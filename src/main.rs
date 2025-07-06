use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

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
        let mut paths: Vec<_> = path_var.split(if cfg!(windows) { ";" } else { ":" }).collect();
        paths.reverse(); // Search rightmost (last) directory first, so last match wins

        for path in paths {
            let path = Path::new(path);
            if !path.is_absolute() {
                continue;
            }

            let file_path = path.join(cmd);
            if file_path.is_file() {
                return Some(file_path);
            }
        }

        None
    }

    fn process_line(&self, line: &str) -> Vec<String> {
        let mut single = false;
        let mut double = false;
        let mut groups = Vec::new();
        let mut cur = String::new();

        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            if single {
                match ch {
                    '\'' => single = false,
                    _ => cur.push(ch),
                };
            } else if double {
                match ch {
                    '"' => double = false,
                    '\\' => {
                        let Some(ch_next) = chars.next() else {
                            break;
                        };
                        if !['\\', '$', '"'].contains(&ch_next) {
                            cur.push(ch);
                        }
                        cur.push(ch_next);
                    }
                    _ => cur.push(ch),
                };
            } else {
                match ch {
                    '\'' => single = true,
                    '"' => double = true,
                    '\\' => {
                        let Some(ch_next) = chars.next() else {
                            break;
                        };
                        cur.push(ch_next);
                    }
                    ch if ch.is_whitespace() => {
                        if !cur.is_empty() {
                            groups.push(cur);
                            cur = String::new();
                        }
                    }
                    _ => cur.push(ch),
                };
            }
        }

        if !cur.is_empty() {
            groups.push(cur);
        }

        groups
    }

    fn exec_command(&self, cmd: &str, args: &[String]) -> Result<(), String> {
        let mut child = Command::new(cmd)
            .args(args)
            .spawn()
            .map_err(|e| e.to_string())?;

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

            let parts: Vec<_> = self.process_line(input.as_str());
            let [cmd, args @ ..] = &parts[..] else {
                continue;
            };

            let res = if let Some(builtin) = self.find_builtin(cmd) {
                builtin(self, args)
            } else if self.find_command(cmd).is_some() {
                self.exec_command(cmd, args)
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