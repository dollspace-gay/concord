use super::{Digest, SearchQueryPlan, Sha256, Utc};

pub(super) fn parse_search_query(query: &str) -> Result<SearchQueryPlan, String> {
    if query.len() > 1_024 || query.chars().any(char::is_control) {
        return Err("invalid search query".into());
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in query.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quoted || escaped {
        return Err("invalid quoted search term".into());
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut plan = SearchQueryPlan::default();
    let mut text = Vec::new();
    for token in tokens {
        let Some((operator, value)) = token.split_once(':') else {
            text.push(token);
            continue;
        };
        match operator.to_ascii_lowercase().as_str() {
            "from" if !value.is_empty() && plan.sender.is_none() => {
                plan.sender = Some(value.to_owned())
            }
            "in" if !value.is_empty() && plan.channel.is_none() => {
                plan.channel = Some(value.to_owned())
            }
            "has" if value.eq_ignore_ascii_case("attachment") => plan.has_attachment = true,
            "has" if value.eq_ignore_ascii_case("link") => plan.has_link = true,
            "before" if plan.before.is_none() => {
                plan.before = Some(normalize_search_timestamp(value, false)?.0)
            }
            "after" if plan.after.is_none() => {
                let (boundary, date_only) = normalize_search_timestamp(value, true)?;
                plan.after = Some(boundary);
                plan.after_inclusive = date_only;
            }
            "from" | "in" | "has" | "before" | "after" => {
                return Err(format!("invalid {operator}: search filter"));
            }
            _ => text.push(token),
        }
    }
    if !text.is_empty() {
        plan.text = Some(text.join(" "));
    }
    if plan.text.is_none()
        && plan.sender.is_none()
        && plan.channel.is_none()
        && !plan.has_attachment
        && !plan.has_link
        && plan.before.is_none()
        && plan.after.is_none()
    {
        return Err("search query is empty".into());
    }
    Ok(plan)
}

pub(super) fn search_fingerprint(server_id: &str, query: &str, channel_id: Option<&str>) -> String {
    let mut digest = Sha256::new();
    digest.update(server_id.as_bytes());
    digest.update([0]);
    digest.update(query.trim().as_bytes());
    digest.update([0]);
    digest.update(channel_id.unwrap_or_default().as_bytes());
    hex::encode(digest.finalize())
}

pub(super) fn normalize_search_timestamp(
    value: &str,
    next_day: bool,
) -> Result<(String, bool), String> {
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok((timestamp.with_timezone(&Utc).to_rfc3339(), false));
    }
    let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "invalid search timestamp".to_string())?;
    let boundary = if next_day {
        date.succ_opt()
            .ok_or_else(|| "invalid search timestamp".to_string())?
    } else {
        date
    };
    Ok((
        boundary
            .and_time(chrono::NaiveTime::MIN)
            .and_utc()
            .to_rfc3339(),
        true,
    ))
}
