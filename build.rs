#[path = "src/cli/args.rs"]
mod args;

use clap::CommandFactory;

fn main() -> std::io::Result<()> {
    // Generate man page for deezchatz-cli
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let out_dir = std::path::PathBuf::from(out_dir);
    
    let app = args::Cli::command();
    
    let man = clap_mangen::Man::new(app.clone());
    let mut buffer: Vec<u8> = Default::default();
    man.render(&mut buffer)?;
    
    std::fs::write(out_dir.join("deezchatz-cli.1"), buffer)?;
    
    Ok(())
}
