# Rustic - A rust based ORM

added some commands 


* this will initilize a schema.rustic file where you can type modals
```
cargo run init <postgres db url>
```

* this will create table inside the database
``` 
cargo run migrate <migration name>
```


some more commands are there which are in development.
```
cargo run generate

cargo run status

cargo run reset
```



## Example Usage

clone this project 
```
git clone https://github.com/TarunVishwakarma1/rust-orm.git
```

build this project
```
cargo build
```
generate a schema.rustic file and migerations folder in root directory by running
```
cargo run init <postgres_database_url>
```

migrate the tables (do not use table name which already exists in postgres)
```
cargo run migrate <name_of_migration>
```

And thats it, your tables should be created in the database now.

