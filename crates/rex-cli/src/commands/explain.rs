//! `rexlang explain`: print one diagnostic code's registry entry (band,
//! meaning, fix), or list the allocation ledger and every registered
//! code. Codes resolve from the central `rex-diagnostics` registry —
//! numeric surface codes and the driver's camelCase LSP codes alike.

use std::process::ExitCode;

pub(crate) fn run(code: Option<&str>, list: bool) -> anyhow::Result<ExitCode> {
    if list {
        print_list();
        return Ok(ExitCode::SUCCESS);
    }
    let Some(code) = code else {
        anyhow::bail!(
            "no code given — pass a diagnostic code (e.g. `rexlang explain E0104`) or `--list`"
        );
    };
    let Some(entry) = rex_diagnostics::lookup(code) else {
        eprintln!(
            "unknown diagnostic code '{code}' — run `rexlang explain --list` to see every registered code"
        );
        return Ok(ExitCode::FAILURE);
    };
    let band = rex_diagnostics::ledger()
        .iter()
        .find(|band| band.id == entry.band)
        .expect("every entry names a registered band");
    println!(
        "{} — {} ({})",
        entry.code,
        entry.title,
        entry.severity.as_str()
    );
    println!("band: {} — {}", band.id, band.owner);
    println!("{}", entry.meaning);
    println!("fix: {}", entry.fix);
    Ok(ExitCode::SUCCESS)
}

fn print_list() {
    println!("rexlang diagnostic codes — `rexlang explain <code>` prints one entry");
    println!();
    let bands = rex_diagnostics::ledger();
    let id_width = bands.iter().map(|band| band.id.len()).max().unwrap_or(0);
    let range_width = bands.iter().map(|band| band.range.len()).max().unwrap_or(0);
    println!(
        "{:<id_width$}  {:<range_width$}  {:<8}  NEXT FREE",
        "BAND", "RANGE", "SEVERITY"
    );
    for band in bands {
        println!(
            "{:<id_width$}  {:<range_width$}  {:<8}  {}",
            band.id, band.range, band.severity_note, band.next_free
        );
    }
    println!();
    let entries = rex_diagnostics::all();
    let code_width = entries
        .iter()
        .map(|entry| entry.code.len())
        .max()
        .unwrap_or(0);
    for entry in entries {
        println!(
            "{:<code_width$}  {:<7}  {}",
            entry.code,
            entry.severity.as_str(),
            entry.title
        );
    }
}
