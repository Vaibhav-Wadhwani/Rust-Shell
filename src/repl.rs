// repl.rs

use rustyline::error::ReadlineError;
use rustyline::{Editor, Helper, CompletionType, Config};
use std::sync::{Arc, Mutex};
use crate::completion::BuiltinCompleter;
use crate::pipeline::execute_pipeline;
use crate::history;

pub fn start_repl() {
    let config = Config::builder().completion_type(CompletionType::List).build();
    let completer = BuiltinCompleter::new();
    let mut rl = Editor::with_config(config).expect("Failed to create Editor");
    rl.set_helper(Some(&completer));
    let history = Arc::new(Mutex::new(Vec::new()));
    // Load history from HISTFILE if set
    if let Ok(histfile) = std::env::var("HISTFILE") {
        if let Ok(file) = std::fs::File::open(&histfile) {
            let reader = std::io::BufReader::new(file);
            let mut hist = history.lock().unwrap();
            for line in reader.lines().flatten() {
                if !line.trim().is_empty() {
                    hist.push(line);
                }
            }
        }
    }
    loop {
        let readline = rl.readline("$ ");
        if let Some(helper) = rl.helper() {
            let c = helper as &BuiltinCompleter;
            let tab_count = *c.tab_count.borrow();
            let matches = c.last_matches.borrow().clone();
            let prefix = c.last_prefix.borrow().clone();
            if matches.len() > 1 && !prefix.is_empty() {
                if tab_count == 1 {
                    print!("\x07");
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                } else if tab_count == 2 {
                    let mut sorted_matches = matches.clone();
                    sorted_matches.sort();
                    println!("{}", sorted_matches.join("  "));
                    print!("$ {}", prefix);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    *c.tab_count.borrow_mut() = 0;
                }
            }
        }
        match readline {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                {
                    let mut hist = history.lock().unwrap();
                    hist.push(trimmed.to_string());
                }
                execute_pipeline(&line, &history);
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
} 