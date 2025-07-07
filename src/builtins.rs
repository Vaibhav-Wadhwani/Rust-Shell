// builtins.rs

use std::env;
use std::sync::{Arc, Mutex};
use crate::util::writeln_ignore_broken_pipe;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::io::BufRead;

pub fn run_builtin(tokens: Vec<String>, history: &Arc<Mutex<Vec<String>>>) {
    if tokens.is_empty() { return; }
    let command = tokens[0].as_str();
    match command {
        "exit" => std::process::exit(
            tokens.get(1)
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(255),
        ),
        "echo" => {
            let output = tokens[1..].join(" ");
            let _ = writeln_ignore_broken_pipe(std::io::stdout(), &output);
            let _ = std::io::stdout().flush();
        }
        "type" => {
            if tokens.len() < 2 {
                return;
            }
            match tokens[1].as_str() {
                "echo" | "exit" | "type" | "pwd" | "cd" | "history" => {
                    println!("{} is a shell builtin", tokens[1])
                }
                _ => {
                    let path = env::var("PATH").unwrap_or_default();
                    let paths = path.split(':');
                    for path in paths {
                        let full_path = format!("{}/{}", path, tokens[1]);
                        if let Ok(metadata) = std::fs::metadata(&full_path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                println!("{} is {}", tokens[1], full_path);
                                return;
                            }
                        }
                    }
                    println!("{}: not found", tokens[1])
                }
            }
        }
        "pwd" => {
            let current = env::current_dir();
            match current {
                Ok(path) => { let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("{}", path.display())); },
                Err(_e) => { let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("{}: command not found", command)); },
            }
        }
        "cd" => {
            if tokens.len() < 2 {
                let _ = writeln_ignore_broken_pipe(std::io::stdout(), "cd: missing argument");
                return;
            }
            let mut target = tokens[1].to_string();
            if target == "~" || target.starts_with("~/") {
                if let Some(home) = env::var_os("HOME") {
                    if target == "~" {
                        target = home.to_string_lossy().to_string();
                    } else {
                        target = format!("{}/{}", home.to_string_lossy(), &target[2..]);
                    }
                }
            }
            if let Err(_) = env::set_current_dir(&target) {
                let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("cd: {}: No such file or directory", tokens[1]));
            }
        }
        "history" => {
            if tokens.len() == 3 && tokens[1] == "-r" {
                let path = &tokens[2];
                if let Ok(file) = std::fs::File::open(path) {
                    let reader = std::io::BufReader::new(file);
                    let mut hist = history.lock().unwrap();
                    for line in reader.lines().flatten() {
                        if !line.trim().is_empty() {
                            hist.push(line);
                        }
                    }
                }
                return;
            }
            // Implement history -w <file>
            if tokens.len() == 3 && tokens[1] == "-w" {
                let path = &tokens[2];
                let mut hist = history.lock().unwrap();
                let this_cmd = tokens.join(" ");
                // Only add if not already the last entry
                let needs_push = hist.last().map(|e| e != &this_cmd).unwrap_or(true);
                if needs_push {
                    hist.push(this_cmd.clone());
                }
                let mut file = match std::fs::File::create(path) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("history: cannot write: {}", e));
                        return;
                    }
                };
                for entry in hist.iter() {
                    let _ = writeln!(file, "{}", entry);
                }
                // Ensure trailing newline (already added by writeln!)
                return;
            }
            // Implement history -a <file>
            if tokens.len() == 3 && tokens[1] == "-a" {
                use std::collections::HashMap;
                use std::sync::OnceLock;
                static LAST_A_IDX: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
                let last_a_idx = LAST_A_IDX.get_or_init(|| Mutex::new(HashMap::new()));
                let path = tokens[2].clone();
                let mut hist = history.lock().unwrap();
                let this_cmd = tokens.join(" ");
                // Only add if not already the last entry
                let needs_push = hist.last().map(|e| e != &this_cmd).unwrap_or(true);
                if needs_push {
                    hist.push(this_cmd.clone());
                }
                let mut last_idx_map = last_a_idx.lock().unwrap();
                let start = *last_idx_map.get(&path).unwrap_or(&0);
                let mut file = match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = writeln_ignore_broken_pipe(std::io::stdout(), &format!("history: cannot append: {}", e));
                        return;
                    }
                };
                for entry in hist.iter().skip(start) {
                    let _ = writeln!(file, "{}", entry);
                }
                last_idx_map.insert(path, hist.len());
                // Ensure trailing newline (already added by writeln!)
                return;
            }
            let hist = history.lock().unwrap();
            if tokens.len() == 2 {
                if let Ok(n) = tokens[1].parse::<usize>() {
                    let total = hist.len();
                    let start = if n > total { 0 } else { total - n };
                    for (i, cmd) in hist.iter().enumerate().skip(start) {
                        println!("{:>5}  {}", i + 1, cmd);
                    }
                    return;
                }
            }
            for (i, cmd) in hist.iter().enumerate() {
                println!("{:>5}  {}", i + 1, cmd);
            }
        }
        _ => unreachable!(),
    }
} 