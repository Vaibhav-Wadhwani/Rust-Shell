#[allow(unused_imports)]
use std::io::{self, Write};
use std::process::exit;

fn main() -> ! {
    loop {
        print!("$ ");
        io::stdout().flush().unwrap();

        let stdin = io::stdin();
        let mut input = String::new();
        stdin.read_line(&mut input).unwrap();

        let input = input.trim();

        let command: Vec<&str> = input.split(" ").collect();

        match command.as_slice() {
            [""] => continue,
            ["echo", args @ ..] => cmd_echo(args),
            ["type", args @ ..] => cmd_type(args),
            ["exit", "0"] => exit(0),
            _ => println!("{}: command not found", input),
        }
    }
}

fn cmd_echo(args: &[&str]) {
    println!("{}", args.join(" "));
}

fn cmd_type(args: &[&str]) {
    let args_len = args.len();

    if args_len == 0 {
        return;
    }

    if args_len > 1 {
        println!("type: too many arguments");
        return;
    }

    match args[0] {
        "type" | "echo" | "exit" => println!("{} is a shell builtin", args[0]),
        _ => println!("{}: not found", args[0]),
    }
}