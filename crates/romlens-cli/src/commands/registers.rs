//! `registers`: the built-in hardware register table.

use anyhow::{Result, anyhow};
use romlens_core::model::{all_hardware_registers, hardware_register};
use romlens_core::{AddressExpr, parse_address_expr};

pub fn run(address: Option<&str>) -> Result<()> {
    match address {
        Some(text) => {
            let a = match parse_address_expr(text)? {
                AddressExpr::Snes(a) => a.offset(),
                AddressExpr::File(_) => return Err(anyhow!("give a CPU address such as $420D")),
            };
            let r = hardware_register(a).ok_or_else(|| anyhow!("no register at ${a:04X}"))?;
            println!(
                "${:04X}  {:<9} {:<3} {}",
                r.address,
                r.name,
                r.access.as_str(),
                r.description
            );
        }
        None => {
            for r in all_hardware_registers() {
                println!(
                    "${:04X}  {:<9} {:<3} {}",
                    r.address,
                    r.name,
                    r.access.as_str(),
                    r.description
                );
            }
        }
    }
    Ok(())
}
