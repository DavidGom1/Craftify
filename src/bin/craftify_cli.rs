//! Command line: craftify-cli <assembly.SLDASM | part.SLDPRT | design.f3d | project.f3z>
//! [--out DIR] [--up y|z] [--solidcraft-cli PATH] [--cadmpeg PATH] [--no-picture] [--check]
//!
//! The same arguments as the first, Python version of Craftify.

use std::path::PathBuf;
use std::process::ExitCode;

use craftify::converter::{self, Hooks, Options};

const USAGE: &str = "usage: craftify-cli SOURCE [--out DIR] [--up y|z] [--solidcraft-cli PATH] [--cadmpeg PATH] [--no-picture] [--check]

Convert SolidWorks assemblies and parts, and Fusion designs, into SolidCraft designs.

  SOURCE             the .SLDASM, .SLDPRT, .f3d or .f3z file to convert
  --out DIR          output folder (default: ./out)
  --up y|z           SolidWorks: turn Y-up models upright (y, the default) or keep them as
                     they are (z); Fusion designs keep their own axes
  --solidcraft-cli   path of solidcraft-cli.exe
  --cadmpeg          path of cadmpeg.exe
  --no-picture       do not draw a picture of the result
  --check            Fusion: have SolidCraft compute each part rebuilt from its timeline and
                     check it against Fusion's (slow); without it the design is written as is";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut source: Option<PathBuf> = None;
    let mut out = std::env::current_dir().unwrap_or_default().join("out");
    let mut up = "y".to_owned();
    let (mut cli, mut cadmpeg) = (None, None);
    let mut picture = true;
    let mut check = false;
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let r = match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--out" => value("--out").map(|v| out = PathBuf::from(v)),
            "--up" => value("--up").map(|v| up = v),
            "--solidcraft-cli" => value("--solidcraft-cli").map(|v| cli = Some(v)),
            "--cadmpeg" => value("--cadmpeg").map(|v| cadmpeg = Some(v)),
            "--no-picture" => {
                picture = false;
                Ok(())
            }
            "--check" => {
                check = true;
                Ok(())
            }
            s if s.starts_with('-') => Err(format!("unrecognized argument: {s}")),
            s if source.is_none() => {
                source = Some(PathBuf::from(s));
                Ok(())
            }
            s => Err(format!("unrecognized argument: {s}")),
        };
        if let Err(e) = r {
            eprintln!("{USAGE}\n\ncraftify-cli: error: {e}");
            return ExitCode::from(2);
        }
    }
    let Some(source) = source.filter(|s| converter::supported(s) && s.is_file()) else {
        eprintln!("{USAGE}\n\ncraftify-cli: error: source must be an existing .SLDASM, .SLDPRT, .f3d or .f3z file");
        return ExitCode::from(2);
    };
    if up != "y" && up != "z" {
        eprintln!("craftify-cli: error: --up must be y or z");
        return ExitCode::from(2);
    }
    let opts = Options { up_y: up == "y", solidcraft_cli: cli.as_deref(), cadmpeg: cadmpeg.as_deref(), picture, check };
    match converter::convert(&source, &out, &opts, &Hooks::print()) {
        Ok(report) if report.ok() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(e) => {
            eprintln!("Error: {e:#}");
            ExitCode::from(1)
        }
    }
}
