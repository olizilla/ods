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

pub struct SuccessionHopLink<'a> {
    pub depth: usize,
    pub date: Option<&'a str>,
    pub code: &'a str,
    pub name: &'a str,
    pub status: &'a str,
}

pub struct RelationshipItemView<'a> {
    pub code: &'a str,
    pub name: &'a str,
    pub status: &'a str,
    pub operational_start: Option<&'a str>,
    pub operational_end: Option<&'a str>,
    pub legal_start: Option<&'a str>,
    pub legal_end: Option<&'a str>,
}

pub struct RelationshipGroup<'a> {
    pub rel_code: &'a str,
    pub rel_name: &'a str,
    pub is_inbound: bool,
    pub items: Vec<RelationshipItemView<'a>>,
    pub total_count: usize,
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
    pub relationships: &'a [RelationshipGroup<'a>],
    pub succession: &'a [SuccessionHopLink<'a>],
    pub predecessors: &'a [SuccessionHopLink<'a>],
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
        let is_active = status.eq_ignore_ascii_case("active");
        if use_color {
            if is_active {
                format!("\x1b[32m{}\x1b[0m", status)
            } else {
                format!("\x1b[2m{}\x1b[0m", status)
            }
        } else {
            status.to_string()
        }
    };

    // Header: # NAME (ODS_CODE)
    writeln!(writer, "# {} ({})", r.name, r.ods_code)?;

    // Core Identity Details
    writeln!(writer, "\n- {}: {}", k("Class"), r.record_class)?;
    writeln!(writer, "- {}: {}", k("Status"), s_val(r.status))?;
    writeln!(
        writer,
        "- {}: {} ({})",
        k("Primary Role"),
        r.role,
        r.role_code
    )?;

    if !r.other_roles.is_empty() {
        writeln!(
            writer,
            "- {}: {}",
            k("Other Roles"),
            r.other_roles.join(", ")
        )?;
    }

    // Lifecycle Dates
    let dates = format_date_range(r.operational_start, r.operational_end, r.status)
        .or_else(|| format_date_range(r.legal_start, r.legal_end, r.status));
    if let Some(dates) = dates {
        writeln!(writer, "- {}: {}", k("Dates"), dates)?;
    }

    if let Some(last_change) = r.last_change_date {
        writeln!(writer, "- {}: {}", k("Last Change"), last_change)?;
    }

    // Contact Details
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
    if !r.relationships.is_empty() {
        writeln!(writer, "\n{}", h("## Relationships"))?;
        for group in r.relationships {
            let group_title = if group.is_inbound {
                format!("{} (Inbound)", group.rel_name)
            } else {
                group.rel_name.to_string()
            };
            writeln!(writer, "\n- {}", k(&group_title))?;

            for item in &group.items {
                let date_str =
                    format_date_range(item.operational_start, item.operational_end, item.status)
                        .or_else(|| {
                            format_date_range(item.legal_start, item.legal_end, item.status)
                        });
                let status_display = s_val(item.status);
                let meta = match date_str {
                    Some(d) => format!("({}, {})", status_display, d),
                    None => format!("({})", status_display),
                };
                if item.name.is_empty() {
                    writeln!(writer, "  {:<10} {}", item.code, meta)?;
                } else {
                    writeln!(writer, "  {:<10} {} {}", item.code, item.name, meta)?;
                }
            }

            if group.total_count > group.items.len() {
                let remaining = group.total_count - group.items.len();
                writeln!(
                    writer,
                    "  ... and {} more (use --format json for all)",
                    remaining
                )?;
            }
        }
    }

    if !r.succession.is_empty() {
        writeln!(writer, "\n{}", h("## Succession"))?;
        for hop in r.succession {
            let date_str = hop.date.unwrap_or("          ");
            let indent = "  ".repeat(hop.depth.saturating_sub(1));
            writeln!(
                writer,
                "  {}  {}→  {:<6}  {:<45}  {}",
                date_str,
                indent,
                hop.code,
                hop.name,
                s_val(hop.status)
            )?;
        }
    }

    if !r.predecessors.is_empty() {
        writeln!(writer, "\n{}", h("## Predecessors"))?;
        for pred in r.predecessors {
            let date_str = pred.date.unwrap_or("          ");
            writeln!(
                writer,
                "  {}  ←  {:<6}  {:<45}  {}",
                date_str,
                pred.code,
                pred.name,
                s_val(pred.status)
            )?;
        }
    }

    Ok(())
}
