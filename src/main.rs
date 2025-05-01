use std::fs;
use std::io::{self};
use std::process::Command;
use structopt::StructOpt;
use regex::Regex;
use std::collections::HashMap;
use postgres::{Client, NoTls};
use rusqlite::Connection;

// Define a simple error type for our ORM
#[derive(Debug)]
enum OrmError {
    Io(io::Error),
    Schema(String),
    Cli(String),
    Migration(String),
    Database(String), // Added for database-related errors
}

// Implement the From trait to convert io::Error to OrmError
impl From<io::Error> for OrmError {
    fn from(error: io::Error) -> Self {
        OrmError::Io(error)
    }
}

// Implement Display for OrmError to provide user-friendly error messages
impl std::fmt::Display for OrmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrmError::Io(err) => write!(f, "IO Error: {}", err),
            OrmError::Schema(msg) => write!(f, "Schema Error: {}", msg),
            OrmError::Cli(msg) => write!(f, "CLI Error: {}", msg),
            OrmError::Migration(msg) => write!(f, "Migration Error: {}", msg),
            OrmError::Database(msg) => write!(f, "Database Error: {}", msg), // Added
        }
    }
}

// Implement Error for OrmError to make it a proper error type
impl std::error::Error for OrmError {}

// Define the CLI arguments using structopt
#[derive(StructOpt, Debug)]
#[structopt(name = "rustic", about = "Rust ORM CLI")]
enum Cli {
    Init {
        #[structopt(help = "Database URL (e.g., sqlite:./dev.db, postgresql://user:pass@host:port/db)")]
        database_url: String,
    },
    Migrate {
        #[structopt(help = "Migration name")]
        name: String,
    },
    Generate,
    Status,
    Reset,
}

// Function to initialize a new project
fn init_project(database_url: &str) -> Result<(), OrmError> {
    // Create a basic schema.rustic file
    let schema_content = format!(
        "datasource \"{}\" {{\n  url = \"{}\"\n}}\n\nmodel Users {{\n  id    Int     @id @default(autoincrement())\n  name  String\n  email String  @unique\n  posts Post[]\n}}\n\nmodel Post {{\n  id        Int     @id @default(autoincrement())\n  title     String\n  content   String\n  author    User    @relation(fields: [authorId], references: [id])\n  authorId  Int\n}}",
        get_datasource_type(database_url), // Extract the type
        database_url
    );
    fs::write("schema.rustic", schema_content)?;

    // Create a migrations directory
    fs::create_dir_all("migrations")?;

    println!("Project initialized.  Edit schema.rustic and run `rustic migrate`.");
    Ok(())
}

// Helper function to extract the datasource type from the URL
fn get_datasource_type(url: &str) -> &str {
    if url.starts_with("sqlite:") {
        "sqlite"
    } else if url.starts_with("postgresql:") || url.starts_with("postgres:") {
        "postgresql"
    } else if url.starts_with("mysql:") {
        "mysql"
    } else {
        "unknown"
    }
}

// Function to generate migrations (more advanced, handles schema diffs)
fn generate_migration(name: &str) -> Result<(), OrmError> {
    let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let migration_name = format!("{}_{}.sql", timestamp, name);
    let migration_path = format!("migrations/{}", migration_name);

    let schema_content = fs::read_to_string("schema.rustic")?;
    let model_regex = Regex::new(r"model\s+(\w+)\s+\{([\s\S]*?)\}")
        .map_err(|e| OrmError::Schema(format!("Failed to create model regex: {}", e)))?;

    // 1. Parse current schema
    let mut current_models = HashMap::new();
    for capture in model_regex.captures_iter(&schema_content) {
        let model_name = capture.get(1).map_or("", |m| m.as_str()).to_string();
        let fields_str = capture.get(2).map_or("", |m| m.as_str()).to_string();
        let mut fields = HashMap::new();
        let field_lines: Vec<&str> = fields_str.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        for field_line in field_lines.iter() {
            let parts: Vec<&str> = field_line.split_whitespace().collect();
            if parts.len() >= 2 {
                let field_name = parts[0].to_string();
                let field_type = parts[1];
                let mut constraints = Vec::new();
                if field_line.contains("@id") {
                    constraints.push("PRIMARY KEY");
                }
                if field_line.contains("@unique") {
                    constraints.push("UNIQUE");
                }
                fields.insert(
                    field_name.clone(),
                    (field_type.to_string(), constraints),
                );
            }
        }
        current_models.insert(model_name, fields);
    }

    // 2.  Load the previous schema (if it exists)
    let previous_models = if let Ok(prev_schema_content) = fs::read_to_string("migrations/previous_schema.rustic") {
        let mut models = HashMap::new();
        for capture in model_regex.captures_iter(&prev_schema_content) {
            let model_name = capture.get(1).map_or("", |m| m.as_str()).to_string();
            let fields_str = capture.get(2).map_or("", |m| m.as_str()).to_string();
             let mut fields = HashMap::new();
            let field_lines: Vec<&str> = fields_str.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            for field_line in field_lines.iter() {
                let parts: Vec<&str> = field_line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let field_name = parts[0].to_string();
                    let field_type = parts[1];
                     let mut constraints = Vec::new();
                    if field_line.contains("@id") {
                        constraints.push("PRIMARY KEY");
                    }
                    if field_line.contains("@unique") {
                        constraints.push("UNIQUE");
                    }
                    fields.insert(
                        field_name.clone(),
                        (field_type.to_string(), constraints),
                    );
                }
            }
            models.insert(model_name, fields);
        }
        models
    } else {
        HashMap::new() // No previous schema
    };

    // 3. Generate SQL diff
    let mut sql_statements = String::new();
    // 3.1 Handle new tables
    for (model_name, fields) in current_models.iter() {
        if !previous_models.contains_key(model_name) {
            sql_statements.push_str(&format!("CREATE TABLE {} (\n", model_name));
            let field_defs: Vec<String> = fields
                .iter()
                .map(|(field_name, (field_type, constraints))| {
                    let sql_type = match field_type.as_str() {
                        "Int" => "INTEGER",
                        "String" => "TEXT",
                        _ => "TEXT",
                    };
                    let mut s = format!("    {} {}", field_name, sql_type);
                    if !constraints.is_empty() {
                        s.push_str(" ");
                        s.push_str(&constraints.join(" "));
                    }
                    s
                })
                .collect();
            sql_statements.push_str(&field_defs.join(",\n"));
            sql_statements.push_str("\n);\n\n");
        } else {
            // 3.2 Handle existing tables (ALTER TABLE)
            let previous_fields = previous_models.get(model_name).unwrap();
            for (field_name, (field_type, constraints)) in fields.iter() {
                if !previous_fields.contains_key(field_name) {
                    // 3.2.1 Add new columns
                    let sql_type = match field_type.as_str() {
                        "Int" => "INTEGER",
                        "String" => "TEXT",
                        _ => "TEXT",
                    };
                    sql_statements.push_str(&format!(
                        "ALTER TABLE {} ADD COLUMN {} {} {}",
                        model_name,
                        field_name,
                        sql_type,
                        if !constraints.is_empty() {
                            constraints.join(" ")
                        } else {
                            "".to_string()
                        }
                    ));
                    sql_statements.push_str(";\n\n");
                } else {
                    // 3.2.2 Check for modifications (type changes, constraints)
                    let (prev_field_type, prev_constraints) = previous_fields.get(field_name).unwrap();
                    if field_type != prev_field_type {
                        //  Simplified:  In a real migration, you'd need to handle data conversion.  This is hard!
                        sql_statements.push_str(&format!(
                            "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                            model_name,
                            field_name,
                            match field_type.as_str() {
                                "Int" => "INTEGER",
                                "String" => "TEXT",
                                _ => "TEXT",
                            }
                        ));
                        sql_statements.push_str(";\n\n");
                    }
                    //Check for constraint changes (simplified)
                    if constraints != prev_constraints{
                         //  This is *very* simplified.  Postgresql has a complex constraint system.
                        if constraints.contains(&&"PRIMARY KEY") && !prev_constraints.contains(&&"PRIMARY KEY"){
                             sql_statements.push_str(&format!(
                                "ALTER TABLE {} ADD CONSTRAINT {}_pk PRIMARY KEY ({})",
                                model_name,
                                field_name,
                                field_name
                            ));
                            sql_statements.push_str(";\n\n");
                        }
                        if !constraints.contains(&&"PRIMARY KEY") && prev_constraints.contains(&&"PRIMARY KEY"){
                             sql_statements.push_str(&format!(
                                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}_pk",
                                model_name,
                                field_name,

                            ));
                            sql_statements.push_str(";\n\n");
                        }
                        if constraints.contains(&&"UNIQUE") && !prev_constraints.contains(&&"UNIQUE"){
                             sql_statements.push_str(&format!(
                                "ALTER TABLE {} ADD CONSTRAINT {}_unique UNIQUE ({})",
                                model_name,
                                field_name,
                                field_name
                            ));
                            sql_statements.push_str(";\n\n");
                        }
                        if !constraints.contains(&&"UNIQUE") && prev_constraints.contains(&&"UNIQUE"){
                             sql_statements.push_str(&format!(
                                "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {}_unique",
                                model_name,
                                field_name,

                            ));
                            sql_statements.push_str(";\n\n");
                        }
                    }
                }
            }
            // 3.3 Handle removed columns
            for field_name in previous_fields.keys() {
                if !fields.contains_key(field_name) {
                    sql_statements.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN {}",
                        model_name, field_name
                    ));
                    sql_statements.push_str(";\n\n");
                }
            }
        }
    }
    // 3.4 Handle removed tables (DROP TABLE)
    for model_name in previous_models.keys() {
        if !current_models.contains_key(model_name) {
            sql_statements.push_str(&format!("DROP TABLE {};\n\n", model_name));
        }
    }

    // 4. Write the SQL to the migration file
    fs::write(&migration_path, sql_statements)?;
    println!("Migration file created: {}", migration_path);

    // 5.  Save the current schema for the next migration.
    fs::write("migrations/previous_schema.rustic", schema_content)?;

    Ok(())
}

// Function to apply migrations (very basic, uses sqlite/psql CLI)
fn apply_migrations() -> Result<(), OrmError> {
    let schema_content = fs::read_to_string("schema.rustic")?;
    let datasource_regex = Regex::new(r#"datasource\s+"(\w+)"\s+\{([\s\S]*?url\s*=\s*"(.*?)")[\s\S]*?\}"#)
        .map_err(|e| OrmError::Schema(format!("Failed to create datasource regex: {}", e)))?;

    let db_url = if let Some(capture) = datasource_regex.captures(&schema_content) {
        capture.get(3).map_or("", |m| m.as_str()).to_string()
    } else {
        return Err(OrmError::Schema("No datasource url found".to_string()));
    };

    println!("Database URL: {}", &db_url);

    let db_type = get_datasource_type(&db_url);

    let migrations = fs::read_dir("migrations")?;
    let mut migration_files: Vec<_> = migrations
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "sql"))
        .collect();
    
    // Sort migrations by filename to ensure correct order
    migration_files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    match db_type {
        "sqlite" => {
            // Extract path from SQLite URL
            let db_path = db_url.trim_start_matches("sqlite:");
            
            // Connect to SQLite database using rusqlite
            let conn = Connection::open(db_path)
                .map_err(|e| OrmError::Database(format!("Failed to connect to SQLite database: {}", e)))?;

            for migration in migration_files {
                let path = migration.path();
                println!("Applying migration: {}", path.display());
                let sql = fs::read_to_string(&path)?;
                
                // Execute SQL batch
                conn.execute_batch(&sql)
                    .map_err(|e| OrmError::Migration(format!(
                        "Failed to apply migration {}: {}", 
                        path.display(), e
                    )))?;
            }
        },
        "postgresql" => {
            // Connect to PostgreSQL database using postgres crate
            let mut client = Client::connect(&db_url, NoTls)
                .map_err(|e| OrmError::Database(format!("Failed to connect to PostgreSQL database: {}", e)))?;
            
            for migration in migration_files {
                let path = migration.path();
                println!("Applying migration: {}", path.display());
                let sql = fs::read_to_string(&path)?;
                
                // Execute the SQL statements
                client.batch_execute(&sql)
                    .map_err(|e| OrmError::Migration(format!(
                        "Failed to apply migration {}: {}", 
                        path.display(), e
                    )))?;
            }
        },
        _ => return Err(OrmError::Database(format!("Unsupported database type: {}", db_type))),
    }

    println!("All migrations applied successfully.");
    Ok(())
}

fn show_status() -> Result<(), OrmError> {
    let schema_content = fs::read_to_string("schema.rustic")?;
    let datasource_regex = Regex::new(r#"datasource\s+"(\w+)"\s+\{([\s\S]*?url\s*=\s*"(.*?)")[\s\S]*?\}"#)
        .map_err(|e| OrmError::Schema(format!("Failed to create datasource regex: {}", e)))?;

    let db_url = if let Some(capture) = datasource_regex.captures(&schema_content) {
        capture.get(3).map_or("", |m| m.as_str()).to_string()
    } else {
        return Err(OrmError::Schema("No datasource url found".to_string()));
    };
    let db_type = get_datasource_type(&db_url);

    println!("Current Status:");
    println!("  Datasource: {}", db_type);
    println!("  URL: {}", db_url);

    // List migrations (very basic - just lists files)
    println!("  Applied Migrations:");
    let migrations = fs::read_dir("migrations")?;
    let mut count = 0;
    for migration in migrations {
        let entry = migration?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "sql") {
            println!("    - {}", path.file_name().unwrap().to_string_lossy());
            count += 1;
        }
    }
    if count == 0 {
        println!("    No migrations applied yet.");
    }

    Ok(())
}

fn reset_database() -> Result<(), OrmError> {
    let schema_content = fs::read_to_string("schema.rustic")?;
    let datasource_regex = Regex::new(r#"datasource\s+"(\w+)"\s+\{([\s\S]*?url\s*=\s*"(.*?)")[\s\S]*?\}"#)
        .map_err(|e| OrmError::Schema(format!("Failed to create datasource regex: {}", e)))?;

    let db_url = if let Some(capture) = datasource_regex.captures(&schema_content) {
        capture.get(3).map_or("", |m| m.as_str()).to_string()
    } else {
        return Err(OrmError::Schema("No datasource url found".to_string()));
    };
    let db_type = get_datasource_type(&db_url);

    match db_type {
        "sqlite" => {
            let db_path = db_url.trim_start_matches("sqlite:");
            if fs::metadata(db_path).is_ok() {
                fs::remove_file(db_path)?;
                println!("Database file '{}' deleted.", db_path);
            } else {
                println!("Database file '{}' not found; nothing to reset.", db_path);
            }
        }
        "postgresql" => {
            // Need to parse the connection string to get the database name.
            let re = Regex::new(r"/([^/]+)$")
                .map_err(|e| OrmError::Database(format!("Failed to create regex: {}", e)))?;
            let db_name = re
                .captures(&db_url)
                .and_then(|cap| cap.get(1).map(|m| m.as_str()))
                .ok_or_else(|| OrmError::Database("Could not parse database name from URL".to_string()))?;

            // Use psql command to drop the database
            let mut cmd = Command::new("psql");
            cmd.arg(&db_url);
            cmd.arg("-c");
            cmd.arg(format!("DROP DATABASE IF EXISTS {}", db_name));

            cmd.stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());

            let status = cmd.status()?;
            if !status.success() {
                return Err(OrmError::Database(format!("Failed to drop database: {}", db_name)));
            }
            println!("Database '{}' dropped.", db_name);

             // Create the database
            let mut create_cmd = Command::new("psql");
            create_cmd.arg(&db_url);
            create_cmd.arg("-c");
            create_cmd.arg(format!("CREATE DATABASE {}", db_name));
             create_cmd.stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
            let create_status = create_cmd.status()?;
             if !create_status.success() {
                return Err(OrmError::Database(format!("Failed to create database: {}", db_name)));
            }
            println!("Database '{}' created.", db_name);
        }
        _ => {
            return Err(OrmError::Database(format!("Unsupported database type: {}", db_type)));
        }
    }

    // Re-apply migrations to re-create the database structure
    apply_migrations()?;
    println!("Database reset and migrations re-applied.");
    Ok(())
}

// Function to generate Rust code from the schema (very basic placeholder)
fn generate_code() -> Result<(), OrmError> {
    //  In a real ORM, this would parse the schema and generate Rust structs
    //  representing the models, along with database access code.
    println!("Generating Rust code from schema.rustic...");
    //  For now, just create a dummy file.
    fs::write("generated.rs", "// This file would contain generated Rust code.\n")?;
    Ok(())
}

fn main() -> Result<(), OrmError> {
    let args = Cli::from_args();

    match args {
        Cli::Init { database_url } => init_project(&database_url)?,
        Cli::Migrate { name } => {
            generate_migration(&name)?;
            apply_migrations()?;
        }
        Cli::Generate => generate_code()?,
        Cli::Status => show_status()?,
        Cli::Reset => reset_database()?,
    }

    Ok(())
}

