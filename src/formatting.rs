use std::io::Write;

pub fn format_date_range(start: Option<&str>, end: Option<&str>, status: &str) -> Option<String> {
    let start = start?;
    let range = match end {
        Some(e) => format!("{start} to {e}"),
        None => {
            if status.eq_ignore_ascii_case("active") {
                format!("{start} to present")
            } else {
                start.to_string()
            }
        }
    };
    Some(range)
}

pub struct SuccessorLink<'a> {
    pub successor_code: &'a str,
    pub successor_name: Option<&'a str>,
}

pub struct InspectorRecord<'a> {
    pub ods_code: &'a str,
    pub name: &'a str,
    pub record_class: &'a str,
    pub status: &'a str,
    pub role: &'a str,
    pub role_code: &'a str,
    pub other_roles: &'a [String],
    pub address: &'a str,
    pub country: &'a str,
    pub uprn: &'a str,
    pub telephone: &'a str,
    pub website: &'a str,
    pub commissioner: &'a str,
    pub commissioner_code: &'a str,
    pub parent: &'a str,
    pub parent_code: &'a str,
    pub pcn: &'a str,
    pub pcn_code: &'a str,
    pub trust: &'a str,
    pub trust_code: &'a str,
    pub icb: &'a str,
    pub icb_code: &'a str,
    pub region: &'a str,
    pub region_code: &'a str,
    pub successors: &'a [SuccessorLink<'a>],
    pub operational_start: Option<&'a str>,
    pub operational_end: Option<&'a str>,
    pub legal_start: Option<&'a str>,
    pub legal_end: Option<&'a str>,
    pub last_change_date: Option<&'a str>,
}

pub fn render_inspector_markdown<W: Write + ?Sized>(
    r: &InspectorRecord,
    use_color: bool,
    writer: &mut W,
) -> std::io::Result<()> {
    let k = |label: &str| -> String {
        if use_color {
            format!("\x1b[1;34m{}\x1b[0m", label)
        } else {
            label.to_string()
        }
    };

    let h = |heading: &str| -> String {
        if use_color {
            format!("\x1b[1;36m{}\x1b[0m", heading)
        } else {
            heading.to_string()
        }
    };

    let s_val = |status: &str| -> String {
        if use_color {
            if status.eq_ignore_ascii_case("active") {
                format!("\x1b[1;32m{}\x1b[0m", status)
            } else {
                format!("\x1b[1;31m{}\x1b[0m", status)
            }
        } else {
            status.to_string()
        }
    };

    writeln!(writer, "{}", h(&format!("# {} ({})", r.name, r.ods_code)))?;
    writeln!(writer, "- {}: {}", k("Class"), r.record_class)?;
    writeln!(writer, "- {}: {}", k("Status"), s_val(r.status))?;

    if !r.role_code.is_empty() {
        writeln!(writer, "- {}: {} ({})", k("Primary Role"), r.role, r.role_code)?;
    } else {
        writeln!(writer, "- {}: {}", k("Primary Role"), r.role)?;
    }

    if !r.other_roles.is_empty() {
        writeln!(writer, "- {}: {}", k("Other Roles"), r.other_roles.join(", "))?;
    }

    if let Some(range) = format_date_range(r.operational_start, r.operational_end, r.status) {
        writeln!(writer, "- {}: {}", k("Operational"), range)?;
    }

    if let Some(range) = format_date_range(r.legal_start, r.legal_end, r.status) {
        writeln!(writer, "- {}: {}", k("Legal"), range)?;
    }

    if let Some(modified) = r.last_change_date {
        writeln!(writer, "- {}: {}", k("Last Modified"), modified)?;
    }

    // Contact Details Section
    let has_contact = !r.address.is_empty()
        || !r.country.is_empty()
        || !r.uprn.is_empty()
        || !r.telephone.is_empty()
        || !r.website.is_empty();

    if has_contact {
        writeln!(writer, "\n{}", h("## Contact Details"))?;
        if !r.address.is_empty() {
            writeln!(writer, "- {}: {}", k("Address"), r.address)?;
        }
        if !r.country.is_empty() {
            writeln!(writer, "- {}: {}", k("Country"), r.country)?;
        }
        if !r.uprn.is_empty() {
            writeln!(writer, "- {}: {}", k("UPRN"), r.uprn)?;
        }
        if !r.telephone.is_empty() {
            writeln!(writer, "- {}: {}", k("Telephone"), r.telephone)?;
        }
        if !r.website.is_empty() {
            writeln!(writer, "- {}: {}", k("Website"), r.website)?;
        }
    }

    // Relationships Section
    let mut rel_items = Vec::new();

    if !r.pcn.is_empty() {
        if !r.pcn_code.is_empty() {
            rel_items.push(format!("- {}: {} ({})", k("PCN"), r.pcn, r.pcn_code));
        } else {
            rel_items.push(format!("- {}: {}", k("PCN"), r.pcn));
        }
    }

    if !r.trust.is_empty() {
        if !r.trust_code.is_empty() {
            rel_items.push(format!("- {}: {} ({})", k("Trust"), r.trust, r.trust_code));
        } else {
            rel_items.push(format!("- {}: {}", k("Trust"), r.trust));
        }
    }

    if !r.icb.is_empty() {
        if !r.icb_code.is_empty() {
            rel_items.push(format!("- {}: {} ({})", k("ICB"), r.icb, r.icb_code));
        } else {
            rel_items.push(format!("- {}: {}", k("ICB"), r.icb));
        }
    }

    if !r.region.is_empty() {
        if !r.region_code.is_empty() {
            rel_items.push(format!("- {}: {} ({})", k("Region"), r.region, r.region_code));
        } else {
            rel_items.push(format!("- {}: {}", k("Region"), r.region));
        }
    }

    if !r.commissioner.is_empty() {
        if !r.commissioner_code.is_empty() {
            rel_items.push(format!("- {}: {} ({})", k("Commissioned By"), r.commissioner, r.commissioner_code));
        } else {
            rel_items.push(format!("- {}: {}", k("Commissioned By"), r.commissioner));
        }
    }

    if !r.parent.is_empty() {
        if !r.parent_code.is_empty() {
            rel_items.push(format!("- {}: {} ({})", k("Parent Org"), r.parent, r.parent_code));
        } else {
            rel_items.push(format!("- {}: {}", k("Parent Org"), r.parent));
        }
    }

    if r.status.eq_ignore_ascii_case("inactive") {
        for s in r.successors {
            let target_name = s.successor_name.unwrap_or("");
            let target_desc = if target_name.is_empty() {
                s.successor_code.to_string()
            } else {
                format!("{} ({target_name})", s.successor_code)
            };
            rel_items.push(format!("- {}: {}", k("Succeeded By"), target_desc));
        }
    }

    if !rel_items.is_empty() {
        writeln!(writer, "\n{}", h("## Relationships"))?;
        for item in rel_items {
            writeln!(writer, "{}", item)?;
        }
    }

    Ok(())
}

