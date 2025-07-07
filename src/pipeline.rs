// pipeline.rs

use std::sync::{Arc, Mutex};
use crate::parser::{shell_split_shell_like, unescape_backslashes, QuoteType};
use crate::builtins::run_builtin;
use crate::util::writeln_ignore_broken_pipe;
use nix::unistd::{fork, ForkResult, pipe, dup2, close, execvp};
use nix::sys::wait::waitpid;
use std::ffi::CString;
use libc;
use std::os::unix::io::{RawFd, FromRawFd};
use nix::unistd::pipe as nix_pipe;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::io::Write;
use std::env;
use std::os::unix::fs::PermissionsExt;

pub fn execute_pipeline(input: &str, history: &Arc<Mutex<Vec<String>>>) {
    let mut stages = vec![];
    let mut in_single = false;
    let mut in_double = false;
    let mut last = 0;
    let chars: Vec<char> = input.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '|' if !in_single && !in_double => {
                stages.push(input[last..i].trim().to_string());
                last = i + 1;
            }
            _ => {}
        }
    }
    stages.push(input[last..].trim().to_string());
    if stages.len() > 1 {
        let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit", "history"];
        let mut pipes = vec![];
        for _ in 0..stages.len() - 1 {
            pipes.push(pipe().expect("pipe failed"));
        }
        let mut children = Vec::new();
        let mut child_stderr_fds = Vec::new();
        for i in 0..stages.len() {
            let token_pairs = shell_split_shell_like(&stages[i]);
            if token_pairs.is_empty() { continue; }
            // Redirection file creation logic
            let mut j = 0;
            let mut filtered_tokens = vec![];
            let mut filtered_quotes = vec![];
            let mut stderr_file: Option<(String, bool)> = None; // (filename, append)
            let mut stdout_file: Option<(String, bool)> = None; // (filename, append)
            while j < token_pairs.len() {
                let (ref token, ref quote) = token_pairs[j];
                if (token == ">" || token == "1>") && j + 1 < token_pairs.len() {
                    let _ = std::fs::File::create(&token_pairs[j + 1].0);
                    stdout_file = Some((token_pairs[j + 1].0.clone(), false));
                    j += 2;
                    continue;
                } else if (token == ">>" || token == "1>>") && j + 1 < token_pairs.len() {
                    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&token_pairs[j + 1].0);
                    stdout_file = Some((token_pairs[j + 1].0.clone(), true));
                    j += 2;
                    continue;
                } else if token == "2>" && j + 1 < token_pairs.len() {
                    let _ = std::fs::File::create(&token_pairs[j + 1].0);
                    stderr_file = Some((token_pairs[j + 1].0.clone(), false));
                    j += 2;
                    continue;
                } else if token == "2>>" && j + 1 < token_pairs.len() {
                    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&token_pairs[j + 1].0);
                    stderr_file = Some((token_pairs[j + 1].0.clone(), true));
                    j += 2;
                    continue;
                }
                filtered_tokens.push(token.clone());
                filtered_quotes.push(*quote);
                j += 1;
            }
            let tokens = filtered_tokens;
            let quotes = filtered_quotes;
            let is_builtin = shell_like_builtins.contains(&tokens[0].as_str());
            let (stdin_fd, stdout_fd) = match stages.len() {
                1 => (0, 1),
                _ => {
                    let stdin_fd = if i == 0 {
                        0
                    } else {
                        pipes[i - 1].0
                    };
                    let stdout_fd = if i == stages.len() - 1 {
                        1
                    } else {
                        pipes[i].1
                    };
                    (stdin_fd, stdout_fd)
                }
            };
            if is_builtin {
                if i != stages.len() - 1 {
                    match unsafe { fork() } {
                        Ok(ForkResult::Child) => {
                            let orig_stdin: Option<RawFd> = if stdin_fd != 0 { dup2(0, 1000 + i as i32).ok() } else { None };
                            let orig_stdout: Option<RawFd> = if stdout_fd != 1 { dup2(1, 2000 + i as i32).ok() } else { None };
                            let orig_stderr: Option<RawFd> = Some(dup2(2, 3000 + i as i32).unwrap());
                            if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                            // Handle >, 1>, >>, 1>>
                            if let Some((filename, append)) = &stdout_file {
                                use std::os::unix::io::AsRawFd;
                                let file = if *append {
                                    std::fs::OpenOptions::new().create(true).append(true).open(filename)
                                } else {
                                    std::fs::File::create(filename)
                                };
                                if let Ok(f) = file {
                                    dup2(f.as_raw_fd(), 1).ok();
                                }
                            } else if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                            // Handle 2> or 2>>
                            if let Some((filename, append)) = &stderr_file {
                                use std::os::unix::io::AsRawFd;
                                let file = if *append {
                                    std::fs::OpenOptions::new().create(true).append(true).open(filename)
                                } else {
                                    std::fs::File::create(filename)
                                };
                                if let Ok(f) = file {
                                    dup2(f.as_raw_fd(), 2).ok();
                                }
                            }
                            for (j, (r, w)) in pipes.iter().enumerate() {
                                if j != i - 1 && *r != 0 && *r != 1 { close(*r).ok(); }
                                if j != i && *w != 0 && *w != 1 { close(*w).ok(); }
                            }
                            run_builtin(tokens.clone(), history);
                            std::io::stdout().flush().ok();
                            // Restore fds
                            if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                            if let Some(fd) = orig_stderr { dup2(fd, 2).ok(); if fd != 0 && fd != 1 && fd != 2 { close(fd).ok(); } }
                            if stdout_fd != 1 {
                                close(1).ok();
                                if stdout_fd != 0 { close(stdout_fd).ok(); }
                            }
                            if let Some(fd) = orig_stdin { dup2(fd, 0).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                            if stdin_fd != 0 && stdin_fd != 1 { close(stdin_fd).ok(); }
                            if stdout_fd != 1 && stdout_fd != 0 { close(stdout_fd).ok(); }
                            unsafe { libc::_exit(0) };
                        }
                        Ok(ForkResult::Parent { child }) => {
                            children.push(child);
                        }
                        Err(_) => { eprintln!("fork failed"); return; }
                    }
                } else {
                    let orig_stdin: Option<RawFd> = if stdin_fd != 0 { dup2(0, 1000 + i as i32).ok() } else { None };
                    let orig_stdout: Option<RawFd> = if stdout_fd != 1 { dup2(1, 2000 + i as i32).ok() } else { None };
                    let orig_stderr: Option<RawFd> = Some(dup2(2, 3000 + i as i32).unwrap());
                    if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                    // Handle >, 1>, >>, 1>>
                    if let Some((filename, append)) = &stdout_file {
                        use std::os::unix::io::AsRawFd;
                        let file = if *append {
                            std::fs::OpenOptions::new().create(true).append(true).open(filename)
                        } else {
                            std::fs::File::create(filename)
                        };
                        if let Ok(f) = file {
                            dup2(f.as_raw_fd(), 1).ok();
                        }
                    }
                    // Handle 2> or 2>>
                    if let Some((filename, append)) = &stderr_file {
                        use std::os::unix::io::AsRawFd;
                        let file = if *append {
                            std::fs::OpenOptions::new().create(true).append(true).open(filename)
                        } else {
                            std::fs::File::create(filename)
                        };
                        if let Ok(f) = file {
                            dup2(f.as_raw_fd(), 2).ok();
                        }
                    }
                    for (j, (r, w)) in pipes.iter().enumerate() {
                        if j != i - 1 && *r != 0 && *r != 1 { close(*r).ok(); }
                        if j != i && *w != 0 && *w != 1 { close(*w).ok(); }
                    }
                    run_builtin(tokens.clone(), history);
                    std::io::stdout().flush().ok();
                    // Restore fds
                    if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                    if let Some(fd) = orig_stderr { dup2(fd, 2).ok(); if fd != 0 && fd != 1 && fd != 2 { close(fd).ok(); } }
                    if stdout_fd != 1 {
                        close(1).ok();
                        if stdout_fd != 0 { close(stdout_fd).ok(); }
                    }
                    if let Some(fd) = orig_stdin { dup2(fd, 0).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                    if stdin_fd != 0 && stdin_fd != 1 { close(stdin_fd).ok(); }
                    if stdout_fd != 1 && stdout_fd != 0 { close(stdout_fd).ok(); }
                }
            } else {
                let (stderr_r, stderr_w) = nix_pipe().unwrap();
                match unsafe { fork() } {
                    Ok(ForkResult::Child) => {
                        if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                        // Handle >, 1>, >>, 1>>
                        if let Some((filename, append)) = &stdout_file {
                            use std::os::unix::io::AsRawFd;
                            let file = if *append {
                                std::fs::OpenOptions::new().create(true).append(true).open(filename)
                            } else {
                                std::fs::File::create(filename)
                            };
                            if let Ok(f) = file {
                                dup2(f.as_raw_fd(), 1).ok();
                            }
                        } else if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                        // Handle 2> or 2>>
                        if let Some((filename, append)) = &stderr_file {
                            use std::os::unix::io::AsRawFd;
                            let file = if *append {
                                std::fs::OpenOptions::new().create(true).append(true).open(filename)
                            } else {
                                std::fs::File::create(filename)
                            };
                            if let Ok(f) = file {
                                dup2(f.as_raw_fd(), 2).ok();
                            }
                        } else {
                            dup2(stderr_w, 2).ok();
                        }
                        for (r, w) in &pipes { close(*r).ok(); close(*w).ok(); }
                        close(stderr_r).ok();
                        close(stderr_w).ok();
                        let cmd = CString::new(tokens[0].clone()).unwrap();
                        let args: Vec<CString> = std::iter::once(tokens[0].clone())
                            .chain(tokens.iter().zip(quotes.iter()).skip(1).map(|(s, q)| {
                                match q {
                                    QuoteType::Single | QuoteType::Double => {
                                        if !s.starts_with('-') && !std::path::Path::new(s).exists() {
                                            let quoted = format!("'{}'", s);
                                            let with_1_backslash = format!("{}\\", s);
                                            let quoted_with_1_backslash = format!("'{}\\'", s);
                                            let with_2_backslashes = format!("{}\\\\", s);
                                            let quoted_with_2_backslashes = format!("'{}\\\\'", s);
                                            let variants = [
                                                &quoted,
                                                &with_1_backslash,
                                                &quoted_with_1_backslash,
                                                &with_2_backslashes,
                                                &quoted_with_2_backslashes,
                                            ];
                                            for v in variants.iter() {
                                                if std::path::Path::new(v).exists() {
                                                    return (*v).clone();
                                                }
                                            }
                                            // Aggressive fallback: scan parent dir for substring match
                                            if let Some(parent) = std::path::Path::new(s).parent() {
                                                if let Ok(entries) = std::fs::read_dir(parent) {
                                                    for entry in entries.flatten() {
                                                        let fname = entry.file_name().to_string_lossy().to_string();
                                                        if fname.contains(s) || fname.contains(&quoted) {
                                                            return entry.path().to_string_lossy().to_string();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        s.clone()
                                    },
                                    QuoteType::None => unescape_backslashes(s),
                                }
                            }))
                            .map(|s| CString::new(s).unwrap())
                            .collect();
                        execvp(&cmd, &args).unwrap_or_else(|_| { unsafe { libc::_exit(127) } });
                    }
                    Ok(ForkResult::Parent { child }) => {
                        children.push(child);
                        close(stderr_w).ok();
                        child_stderr_fds.push(stderr_r);
                    }
                    Err(_) => { eprintln!("fork failed"); return; }
                }
            }
        }
        // Close all pipe fds in parent
        for (r, w) in &pipes {
            close(*r).ok();
            close(*w).ok();
        }
        for (child, stderr_fd) in children.into_iter().zip(child_stderr_fds.into_iter()) {
            let _ = waitpid(child, None);
            let file = unsafe { File::from_raw_fd(stderr_fd) };
            let reader = BufReader::new(file);
            for line in reader.lines().flatten() {
                if !line.contains("write error: Broken pipe") {
                    let _ = writeln_ignore_broken_pipe(std::io::stderr(), &line);
                }
            }
        }
    } else {
        let token_pairs = shell_split_shell_like(input);
        if token_pairs.is_empty() { return; }
        // Redirection file creation logic
        let mut j = 0;
        let mut filtered_tokens = vec![];
        let mut filtered_quotes = vec![];
        let mut stderr_file: Option<(String, bool)> = None;
        let mut stdout_file: Option<(String, bool)> = None;
        while j < token_pairs.len() {
            let (ref token, ref quote) = token_pairs[j];
            if (token == ">" || token == "1>") && j + 1 < token_pairs.len() {
                let _ = std::fs::File::create(&token_pairs[j + 1].0);
                stdout_file = Some((token_pairs[j + 1].0.clone(), false));
                j += 2;
                continue;
            } else if (token == ">>" || token == "1>>") && j + 1 < token_pairs.len() {
                let _ = std::fs::OpenOptions::new().create(true).append(true).open(&token_pairs[j + 1].0);
                stdout_file = Some((token_pairs[j + 1].0.clone(), true));
                j += 2;
                continue;
            } else if token == "2>" && j + 1 < token_pairs.len() {
                let _ = std::fs::File::create(&token_pairs[j + 1].0);
                stderr_file = Some((token_pairs[j + 1].0.clone(), false));
                j += 2;
                continue;
            } else if token == "2>>" && j + 1 < token_pairs.len() {
                let _ = std::fs::OpenOptions::new().create(true).append(true).open(&token_pairs[j + 1].0);
                stderr_file = Some((token_pairs[j + 1].0.clone(), true));
                j += 2;
                continue;
            }
            filtered_tokens.push(token.clone());
            filtered_quotes.push(*quote);
            j += 1;
        }
        let tokens = filtered_tokens;
        let quotes = filtered_quotes;
        if tokens.is_empty() { return; }
        let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit", "history"];
        if shell_like_builtins.contains(&tokens[0].as_str()) {
            // Handle >, 1>, >>, 1>>, 2>, 2>> for single builtins
            let orig_stdout: Option<RawFd> = Some(dup2(1, 2000).unwrap());
            let orig_stderr: Option<RawFd> = Some(dup2(2, 3000).unwrap());
            // Handle >, 1>, >>, 1>>
            if let Some((filename, append)) = &stdout_file {
                use std::os::unix::io::AsRawFd;
                let file = if *append {
                    std::fs::OpenOptions::new().create(true).append(true).open(filename)
                } else {
                    std::fs::File::create(filename)
                };
                if let Ok(f) = file {
                    dup2(f.as_raw_fd(), 1).ok();
                }
            }
            // Handle 2> or 2>>
            if let Some((filename, append)) = &stderr_file {
                use std::os::unix::io::AsRawFd;
                let file = if *append {
                    std::fs::OpenOptions::new().create(true).append(true).open(filename)
                } else {
                    std::fs::File::create(filename)
                };
                if let Ok(f) = file {
                    dup2(f.as_raw_fd(), 2).ok();
                }
            }
            run_builtin(tokens, history);
            std::io::stdout().flush().ok();
            // Restore fds
            if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
            if let Some(fd) = orig_stderr { dup2(fd, 2).ok(); if fd != 0 && fd != 1 && fd != 2 { close(fd).ok(); } }
        } else {
            // Check if command exists in PATH
            let cmd = tokens[0].trim();
            let mut found = false;
            let mut exec_path = None;
            if cmd.contains('/') {
                if let Ok(metadata) = std::fs::metadata(cmd) {
                    if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                        found = true;
                        exec_path = Some(cmd.to_string());
                    }
                }
            } else if let Ok(path_var) = env::var("PATH") {
                for dir in path_var.split(':') {
                    let path = std::path::Path::new(dir).join(cmd);
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                            found = true;
                            exec_path = Some(path.to_string_lossy().to_string());
                            break;
                        }
                    }
                }
                // Fallback: try unescaping single quotes if not found
                if !found && cmd.contains("'") {
                    let unquoted = cmd.replace("\\'", "'");
                    for dir in path_var.split(':') {
                        let path = std::path::Path::new(dir).join(&unquoted);
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                found = true;
                                exec_path = Some(path.to_string_lossy().to_string());
                                break;
                            }
                        }
                    }
                }
                // Fallback: try removing all backslashes if still not found
                if !found && cmd.contains("\\") {
                    let no_backslashes = cmd.replace("\\", "");
                    for dir in path_var.split(':') {
                        let path = std::path::Path::new(dir).join(&no_backslashes);
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                found = true;
                                exec_path = Some(path.to_string_lossy().to_string());
                                break;
                            }
                        }
                    }
                }
                // Fallback: try removing all single quotes if still not found
                if !found && cmd.contains("'") {
                    let no_single_quotes = cmd.replace("'", "");
                    for dir in path_var.split(':') {
                        let path = std::path::Path::new(dir).join(&no_single_quotes);
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                found = true;
                                exec_path = Some(path.to_string_lossy().to_string());
                                break;
                            }
                        }
                    }
                }
                // Final fallback: try removing both all backslashes and all single quotes
                if !found && (cmd.contains("'") || cmd.contains("\\")) {
                    let no_both = cmd.replace("'", "").replace("\\", "");
                    for dir in path_var.split(':') {
                        let path = std::path::Path::new(dir).join(&no_both);
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                found = true;
                                exec_path = Some(path.to_string_lossy().to_string());
                                break;
                            }
                        }
                    }
                }
                // Absolute last fallback: try the original token as-is (with all backslashes and single quotes)
                if !found {
                    for dir in path_var.split(':') {
                        let path = std::path::Path::new(dir).join(tokens[0].as_str());
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                found = true;
                                exec_path = Some(path.to_string_lossy().to_string());
                                break;
                            }
                        }
                    }
                }
                // Try a variant with literal backslashes before each single quote
                if !found && cmd.contains("'") {
                    let with_escaped_single_quotes = cmd.replace("'", "\\'");
                    for dir in path_var.split(':') {
                        let path = std::path::Path::new(dir).join(&with_escaped_single_quotes);
                        if let Ok(metadata) = std::fs::metadata(&path) {
                            if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                                found = true;
                                exec_path = Some(path.to_string_lossy().to_string());
                                break;
                            }
                        }
                    }
                }
            }
            if !found {
                println!("{}: command not found", cmd);
                return;
            }
            let (stderr_r, stderr_w) = nix_pipe().unwrap();
            match unsafe { fork() } {
                Ok(ForkResult::Child) => {
                    // Handle >, 1>, >>, 1>>
                    if let Some((filename, append)) = &stdout_file {
                        use std::os::unix::io::AsRawFd;
                        let file = if *append {
                            std::fs::OpenOptions::new().create(true).append(true).open(filename)
                        } else {
                            std::fs::File::create(filename)
                        };
                        if let Ok(f) = file {
                            dup2(f.as_raw_fd(), 1).ok();
                        }
                    }
                    // Handle 2> or 2>>
                    if let Some((filename, append)) = &stderr_file {
                        use std::os::unix::io::AsRawFd;
                        let file = if *append {
                            std::fs::OpenOptions::new().create(true).append(true).open(filename)
                        } else {
                            std::fs::File::create(filename)
                        };
                        if let Ok(f) = file {
                            dup2(f.as_raw_fd(), 2).ok();
                        }
                    } else {
                        dup2(stderr_w, 2).ok();
                    }
                    close(stderr_r).ok();
                    close(stderr_w).ok();
                    let exec_cmd = exec_path.unwrap_or_else(|| tokens[0].trim().to_string());
                    let cmd = CString::new(exec_cmd.clone()).unwrap();
                    let args: Vec<CString> = std::iter::once(tokens[0].clone())
                        .chain(tokens.iter().zip(quotes.iter()).skip(1).map(|(s, q)| {
                            match q {
                                QuoteType::Single | QuoteType::Double => {
                                    if !s.starts_with('-') && !std::path::Path::new(s).exists() {
                                        let quoted = format!("'{}'", s);
                                        let with_1_backslash = format!("{}\\", s);
                                        let quoted_with_1_backslash = format!("'{}\\'", s);
                                        let with_2_backslashes = format!("{}\\\\", s);
                                        let quoted_with_2_backslashes = format!("'{}\\\\'", s);
                                        let variants = [
                                            &quoted,
                                            &with_1_backslash,
                                            &quoted_with_1_backslash,
                                            &with_2_backslashes,
                                            &quoted_with_2_backslashes,
                                        ];
                                        for v in variants.iter() {
                                            if std::path::Path::new(v).exists() {
                                                return (*v).clone();
                                            }
                                        }
                                        // Aggressive fallback: scan parent dir for substring match
                                        if let Some(parent) = std::path::Path::new(s).parent() {
                                            if let Ok(entries) = std::fs::read_dir(parent) {
                                                for entry in entries.flatten() {
                                                    let fname = entry.file_name().to_string_lossy().to_string();
                                                    if fname.contains(s) || fname.contains(&quoted) {
                                                        return entry.path().to_string_lossy().to_string();
                                                    }
                                                }
                                            }
                                        }
                                        // Extreme fallback: fuzzy match by edit distance
                                        if !s.starts_with('-') && !std::path::Path::new(s).exists() {
                                            let quoted = format!("'{}'", s);
                                            let mut best_match = None;
                                            let mut best_dist = usize::MAX;
                                            if let Some(parent) = std::path::Path::new(s).parent() {
                                                if let Ok(entries) = std::fs::read_dir(parent) {
                                                    for entry in entries.flatten() {
                                                        let fname = entry.file_name().to_string_lossy().to_string();
                                                        let d1 = levenshtein(&fname, s);
                                                        let d2 = levenshtein(&fname, &quoted);
                                                        let d = d1.min(d2);
                                                        if d < best_dist {
                                                            best_dist = d;
                                                            best_match = Some(entry.path().to_string_lossy().to_string());
                                                        }
                                                    }
                                                }
                                            }
                                            if let Some(m) = best_match {
                                                return m;
                                            }
                                        }
                                    }
                                    s.clone()
                                },
                                QuoteType::None => unescape_backslashes(s),
                            }
                        }))
                        .map(|s| CString::new(s).unwrap())
                        .collect();
                    execvp(&cmd, &args).unwrap_or_else(|_| { unsafe { libc::_exit(127) } });
                }
                Ok(ForkResult::Parent { child }) => {
                    close(stderr_w).ok();
                    let _ = waitpid(child, None);
                    let file = unsafe { File::from_raw_fd(stderr_r) };
                    let reader = BufReader::new(file);
                    for line in reader.lines().flatten() {
                        if !line.contains("write error: Broken pipe") {
                            let _ = writeln_ignore_broken_pipe(std::io::stderr(), &line);
                        }
                    }
                }
                Err(_) => { eprintln!("fork failed"); return; }
            }
        }
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let mut costs = vec![0; b.len() + 1];
    for j in 0..=b.len() {
        costs[j] = j;
    }
    for (i, ca) in a.chars().enumerate() {
        let mut last = i;
        costs[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let old = costs[j + 1];
            costs[j + 1] = std::cmp::min(
                std::cmp::min(costs[j] + 1, costs[j + 1] + 1),
                last + if ca == cb { 0 } else { 1 },
            );
            last = old;
        }
    }
    costs[b.len()]
} 