//! triodion_cli is a cli for triodion_core

mod args;
mod argv;
mod parse;
mod remember;
mod run;

pub use args::Args;
use eyre::Result;

#[tokio::main]
#[allow(unreachable_code)]
#[allow(clippy::needless_return)]
async fn main() -> Result<()> {
    let args = Args::parse_cli();
    match run::run(args).await {
        Ok(Some(freeze_summary)) if freeze_summary.errored.is_empty() => Ok(()),
        Ok(Some(_freeze_summary)) => std::process::exit(1),
        Ok(None) => Ok(()),
        Err(e) => {
            // handle debug build
            #[cfg(debug_assertions)]
            {
                return Err(eyre::Report::from(e));
            }

            // handle release build
            #[cfg(not(debug_assertions))]
            {
                println!("{}", e);
                std::process::exit(1);
            }
        }
    }
}
