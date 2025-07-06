use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{exit, Command};
use std::os::unix::io::{AsRawFd, FromRawFd};

type Builtin = fn(&mut Shell, &[String]) -> Result<(), String>;

struct Shell {
    builtins: HashMap<&'static str, Builtin>,
    cwd: PathBuf,
}

impl Shell {
    fn try_new() -> Option<Shell> {
        let builtins = HashMap::from([
            ("type", Shell::builtin_type as Builtin),
            ("exit", Shell::builtin_exit as Builtin),
            ("echo", Shell::builtin_echo as Builtin),
            ("pwd", Shell::builtin_pwd as Builtin),
            ("cd", Shell::builtin_cd as Builtin),
        ]);

        let cwd = std::env::current_dir().ok()?;

        Some(Shell { builtins, cwd })
    }

    fn builtin_echo(&mut self, args: &[String]) -> Result<(), String> {
        println!("{}", args.join(" "));
        Ok(())
    }

    fn builtin_exit(&mut self, args: &[String]) -> Result<(), String> {
        let code = match args {
            [] => 0,
            [arg] => arg
                .parse::<i32>()
                .map_err(|_| format!("invalid return code {arg}"))?,
            _ => return Err("invalid number of arguments".into()),
        };

        exit(code);
    }

    fn builtin_type(&mut self, args: &[String]) -> Result<(), String> {
        let [arg] = args else {
            return Err("invalid number of arguments".into());
        };

        if self.find_builtin(&arg).is_some() {
            println!("{arg} is a shell builtin");
            return Ok(());
        }

        if let Some(path) = self.find_command(&arg) {
            println!("{arg} is {}", path.display());
            return Ok(());
        }

        println!("{arg}: not found");
        Ok(())
    }

    fn builtin_pwd(&mut self, _: &[String]) -> Result<(), String> {
        println!("{}", self.cwd.display());
        Ok(())
    }

    fn builtin_cd(&mut self, args: &[String]) -> Result<(), String> {
        let [arg] = args else {
            return Err("invalid number of arguments".into());
        };

        let new_cwd = if arg == "~" {
            let home = std::env::var("HOME").map_err(|e| e.to_string())?;
            PathBuf::from(home)
        } else if std::path::Path::new(arg).is_absolute() {
            PathBuf::from(arg)
        } else {
            self.cwd.join(arg)
        };

        if new_cwd.is_file() {
            return Err(format!("not a directory: {arg}"));
        }
        if !new_cwd.is_dir() {
            return Err(format!("{arg}: No such file or directory"));
        }

        std::env::set_current_dir(&new_cwd).map_err(|e| e.to_string())?;
        self.cwd = new_cwd.canonicalize().unwrap_or(new_cwd);
        Ok(())
    }

    fn find_builtin(&self, cmd: &str) -> Option<Builtin> {
        self.builtins.get(cmd).map(|b| *b)
    }

    fn find_command(&self, cmd: &str) -> Option<PathBuf> {
        let path_var = std::env::var("PATH").ok()?;
        let paths = path_var.split(if cfg!(windows) { ";" } else { ":" });

        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        for path in paths {
            if path.is_empty() {
                continue;
            }
            let path = std::path::Path::new(path);
            if !path.is_absolute() {
                continue;
            }
            let file_path = path.join(cmd);
            let exists = file_path.is_file();
            #[cfg(unix)]
            let exec = exists && file_path.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
            #[cfg(not(unix))]
            let exec = exists;
            if exec {
                return Some(file_path);
            }
        }
        None
    }

    fn process_line(&self, line: &str) -> (Vec<String>, Option<String>) {
        let mut tokens = Vec::new();
        let mut redir: Option<String> = None;
        let mut cur = String::new();
        let mut chars = line.chars().peekable();
        let mut single = false;
        let mut double = false;
        while let Some(&ch) = chars.peek() {
            if single {
                chars.next();
                match ch {
                    '\'' => single = false,
                    _ => cur.push(ch),
                }
            } else if double {
                chars.next();
                match ch {
                    '"' => double = false,
                    '\\' => {
                        chars.next();
                        if let Some(&ch_next) = chars.peek() {
                            match ch_next {
                                '\\' | '"' | '$' => {
                                    cur.push(ch_next);
                                    chars.next();
                                }
                                _ => {
                                    cur.push('\\');
                                    cur.push(ch_next);
                                    chars.next();
                                }
                            }
                        } else {
                            cur.push('\\');
                        }
                    }
                    _ => cur.push(ch),
                }
            } else {
                // Redirection: > or 1>
                if ch == '>' || (ch == '1' && chars.clone().nth(1) == Some('>')) {
                    if !cur.is_empty() {
                        tokens.push(cur.clone());
                        cur.clear();
                    }
                    if ch == '1' {
                        chars.next();
                    }
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        if c.is_whitespace() {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let mut fname = String::new();
                    let mut in_single = false;
                    let mut in_double = false;
                    while let Some(&c) = chars.peek() {
                        if !in_single && !in_double && c.is_whitespace() {
                            break;
                        }
                        if c == '\'' && !in_double {
                            in_single = !in_single;
                            chars.next();
                            continue;
                        }
                        if c == '"' && !in_single {
                            in_double = !in_double;
                            chars.next();
                            continue;
                        }
                        fname.push(c);
                        chars.next();
                    }
                    redir = Some(fname);
                } else {
                    match ch {
                        '\'' => { single = true; chars.next(); },
                        '"' => { double = true; chars.next(); },
                        '\\' => {
                            chars.next();
                            if let Some(&ch_next) = chars.peek() {
                                cur.push(ch_next);
                                chars.next();
                            }
                        }
                        c if c.is_whitespace() => {
                            chars.next();
                            if !cur.is_empty() {
                                tokens.push(cur.clone());
                                cur.clear();
                            }
                        }
                        _ => { cur.push(ch); chars.next(); },
                    }
                }
            }
        }
        if !cur.is_empty() {
            tokens.push(cur);
        }
        (tokens, redir)
    }

    fn exec_command(&self, cmd: &str, args: &[String], redir: Option<&String>) -> Result<(), String> {
        let mut command = Command::new(cmd);
        command.args(args);
        if let Some(filename) = redir {
            use std::fs::File;
            let file = File::create(filename).map_err(|e| e.to_string())?;
            command.stdout(std::process::Stdio::from(file));
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn run(&mut self) {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        loop {
            print!("$ ");
            stdout.flush().unwrap();
            let mut input = String::new();
            stdin.read_line(&mut input).unwrap();
            let (parts, redir) = self.process_line(input.as_str());
            let [cmd, args @ ..] = &parts[..] else {
                continue;
            };
            let res = if let Some(builtin) = self.find_builtin(cmd) {
                if let Some(filename) = &redir {
                    use std::fs::File;
                    let file = File::create(filename);
                    match file {
                        Ok(file) => {
                            let stdout_fd = 1; // STDOUT_FILENO
                            let saved = unsafe { libc::dup(stdout_fd) };
                            unsafe { libc::dup2(file.as_raw_fd(), stdout_fd) };
                            let result = builtin(self, args);
                            unsafe { libc::dup2(saved, stdout_fd); libc::close(saved); }
                            result
                        },
                        Err(e) => Err(e.to_string()),
                    }
                } else {
                    builtin(self, args)
                }
            } else if self.find_command(cmd).is_some() {
                self.exec_command(cmd, args, redir.as_ref())
            } else {
                Err("command not found".into())
            };
            if let Err(err) = res {
                println!("{cmd}: {err}");
            }
        }
    }
}

fn main() {
    let mut shell = Shell::try_new().unwrap();
    shell.run();
}