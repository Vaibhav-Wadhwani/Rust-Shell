mod repl;
mod parser;
mod pipeline;
mod builtins;
mod history;
mod completion;
mod util;

fn main() {
    repl::start_repl();
}
