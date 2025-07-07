// pipeline.rs

use std::sync::{Arc, Mutex};
use crate::parser::shell_split_shell_like;
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

pub fn execute_pipeline(input: &str) {
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
            let tokens = shell_split_shell_like(&stages[i]);
            if tokens.is_empty() { continue; }
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
                            if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                            if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                            for (j, (r, w)) in pipes.iter().enumerate() {
                                if j != i - 1 && *r != 0 && *r != 1 { close(*r).ok(); }
                                if j != i && *w != 0 && *w != 1 { close(*w).ok(); }
                            }
                            run_builtin(tokens.clone(), &Arc::new(Mutex::new(Vec::new())));
                            std::io::stdout().flush().ok();
                            if stdout_fd != 1 {
                                close(1).ok();
                                if stdout_fd != 0 { close(stdout_fd).ok(); }
                                if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                            } else {
                                if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
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
                    if stdin_fd != 0 { dup2(stdin_fd, 0).ok(); }
                    if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                    for (j, (r, w)) in pipes.iter().enumerate() {
                        if j != i - 1 && *r != 0 && *r != 1 { close(*r).ok(); }
                        if j != i && *w != 0 && *w != 1 { close(*w).ok(); }
                    }
                    run_builtin(tokens.clone(), &Arc::new(Mutex::new(Vec::new())));
                    std::io::stdout().flush().ok();
                    if stdout_fd != 1 {
                        close(1).ok();
                        if stdout_fd != 0 { close(stdout_fd).ok(); }
                        if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
                    } else {
                        if let Some(fd) = orig_stdout { dup2(fd, 1).ok(); if fd != 0 && fd != 1 { close(fd).ok(); } }
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
                        if stdout_fd != 1 { dup2(stdout_fd, 1).ok(); }
                        dup2(stderr_w, 2).ok();
                        for (r, w) in &pipes { close(*r).ok(); close(*w).ok(); }
                        close(stderr_r).ok();
                        close(stderr_w).ok();
                        let cmd = CString::new(tokens[0].clone()).unwrap();
                        let args: Vec<CString> = tokens.iter().map(|s| CString::new(s.as_str()).unwrap()).collect();
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
        let tokens = shell_split_shell_like(input);
        if tokens.is_empty() { return; }
        let shell_like_builtins = ["echo", "type", "pwd", "cd", "exit", "history"];
        if shell_like_builtins.contains(&tokens[0].as_str()) {
            run_builtin(tokens, &Arc::new(Mutex::new(Vec::new())));
        } else {
            let (stderr_r, stderr_w) = nix_pipe().unwrap();
            match unsafe { fork() } {
                Ok(ForkResult::Child) => {
                    dup2(stderr_w, 2).ok();
                    close(stderr_r).ok();
                    close(stderr_w).ok();
                    let cmd = CString::new(tokens[0].clone()).unwrap();
                    let args: Vec<CString> = tokens.iter().map(|s| CString::new(s.as_str()).unwrap()).collect();
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