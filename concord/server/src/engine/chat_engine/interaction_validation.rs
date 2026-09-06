use super::SlashCommandOption;

pub(super) fn find_message_component<'a>(
    components: &'a [crate::engine::events::MessageComponent],
    custom_id: &str,
) -> Option<&'a crate::engine::events::MessageComponent> {
    for component in components {
        match component {
            crate::engine::events::MessageComponent::ActionRow { components } => {
                if let Some(found) = find_message_component(components, custom_id) {
                    return Some(found);
                }
            }
            crate::engine::events::MessageComponent::Button {
                custom_id: candidate,
                ..
            }
            | crate::engine::events::MessageComponent::SelectMenu {
                custom_id: candidate,
                ..
            } if candidate == custom_id => return Some(component),
            _ => {}
        }
    }
    None
}

pub(super) fn safe_embed_url(value: &str) -> bool {
    if value.len() > 2_048 {
        return false;
    }
    reqwest::Url::parse(value).ok().is_some_and(|url| {
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        let host = url
            .host_str()
            .unwrap()
            .trim_matches(['[', ']'])
            .to_ascii_lowercase();
        if host == "localhost"
            || host.ends_with(".localhost")
            || host.ends_with(".local")
            || host.ends_with(".internal")
        {
            return false;
        }
        match host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(address)) => {
                !(address.is_private()
                    || address.is_loopback()
                    || address.is_link_local()
                    || address.is_unspecified()
                    || address.is_multicast())
            }
            Ok(std::net::IpAddr::V6(address)) => {
                !(address.is_loopback()
                    || address.is_unspecified()
                    || address.is_multicast()
                    || (address.segments()[0] & 0xfe00) == 0xfc00
                    || (address.segments()[0] & 0xffc0) == 0xfe80)
            }
            Err(_) => true,
        }
    })
}

pub(super) fn validate_rich_interaction_response(
    embeds: Option<&[crate::engine::events::RichEmbedInfo]>,
    components: Option<&[crate::engine::events::MessageComponent]>,
) -> Result<(), String> {
    if let Some(embeds) = embeds {
        for embed in embeds {
            if embed.title.as_ref().is_some_and(|value| value.len() > 256)
                || embed
                    .description
                    .as_ref()
                    .is_some_and(|value| value.len() > 4096)
                || embed.color.as_ref().is_some_and(|value| {
                    value.len() != 7
                        || !value.starts_with('#')
                        || !value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                || embed.fields.as_ref().is_some_and(|fields| {
                    fields.len() > 25
                        || fields
                            .iter()
                            .any(|field| field.name.len() > 256 || field.value.len() > 1024)
                })
                || embed
                    .footer
                    .as_ref()
                    .is_some_and(|footer| footer.text.len() > 2048)
                || embed
                    .author
                    .as_ref()
                    .is_some_and(|author| author.name.len() > 256)
            {
                return Err("Invalid interaction embed".into());
            }
            for url in [
                embed.url.as_deref(),
                embed.image_url.as_deref(),
                embed.thumbnail_url.as_deref(),
                embed
                    .footer
                    .as_ref()
                    .and_then(|footer| footer.icon_url.as_deref()),
                embed
                    .author
                    .as_ref()
                    .and_then(|author| author.url.as_deref()),
                embed
                    .author
                    .as_ref()
                    .and_then(|author| author.icon_url.as_deref()),
            ]
            .into_iter()
            .flatten()
            {
                if !safe_embed_url(url) {
                    return Err("Interaction embed URL must use HTTPS".into());
                }
            }
        }
    }
    let mut custom_ids = std::collections::HashSet::new();
    if let Some(components) = components {
        validate_message_components(components, true, &mut custom_ids)?;
    }
    Ok(())
}

pub(super) fn validate_message_components(
    components: &[crate::engine::events::MessageComponent],
    top_level: bool,
    custom_ids: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    for component in components {
        match component {
            crate::engine::events::MessageComponent::ActionRow { components } => {
                if !top_level || components.is_empty() || components.len() > 5 {
                    return Err("Invalid interaction component layout".into());
                }
                validate_message_components(components, false, custom_ids)?;
            }
            crate::engine::events::MessageComponent::Button {
                custom_id,
                label,
                style,
                emoji,
                ..
            } => {
                if top_level
                    || custom_id.is_empty()
                    || custom_id.len() > 100
                    || label.is_empty()
                    || label.len() > 80
                    || emoji.as_ref().is_some_and(|value| value.len() > 64)
                    || !matches!(
                        style.as_str(),
                        "primary" | "secondary" | "success" | "danger"
                    )
                    || !custom_ids.insert(custom_id.clone())
                {
                    return Err("Invalid interaction button".into());
                }
            }
            crate::engine::events::MessageComponent::SelectMenu {
                custom_id,
                placeholder,
                options,
                min_values,
                max_values,
            } => {
                let unique_values: std::collections::HashSet<_> =
                    options.iter().map(|option| option.value.as_str()).collect();
                if top_level
                    || custom_id.is_empty()
                    || custom_id.len() > 100
                    || placeholder.as_ref().is_some_and(|value| value.len() > 150)
                    || options.is_empty()
                    || options.len() > 25
                    || unique_values.len() != options.len()
                    || *min_values < 0
                    || *max_values < 1
                    || min_values > max_values
                    || *max_values as usize > options.len()
                    || options.iter().any(|option| {
                        option.label.is_empty()
                            || option.label.len() > 100
                            || option.value.is_empty()
                            || option.value.len() > 100
                            || option
                                .description
                                .as_ref()
                                .is_some_and(|value| value.len() > 100)
                            || option.emoji.as_ref().is_some_and(|value| value.len() > 64)
                    })
                    || !custom_ids.insert(custom_id.clone())
                {
                    return Err("Invalid interaction select menu".into());
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_slash_command_options(options: &[SlashCommandOption]) -> Result<(), String> {
    if options.len() > 25 {
        return Err("A command may define at most 25 options".into());
    }
    let mut names = std::collections::HashSet::new();
    let mut saw_optional = false;
    for option in options {
        if option.name.is_empty()
            || option.name.len() > 32
            || !option.name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
            || !names.insert(option.name.as_str())
        {
            return Err("Command option names must be unique lowercase identifiers".into());
        }
        if option.description.is_empty()
            || option.description.len() > 100
            || option.description.chars().any(char::is_control)
        {
            return Err("Command option descriptions must be 1-100 printable characters".into());
        }
        if !matches!(
            option.option_type.as_str(),
            "string" | "integer" | "boolean" | "user" | "channel" | "role"
        ) {
            return Err("Unsupported command option type".into());
        }
        if saw_optional && option.required {
            return Err("Required command options must precede optional options".into());
        }
        saw_optional |= !option.required;
        if let Some(choices) = &option.choices {
            if option.option_type != "string" && option.option_type != "integer" {
                return Err("Only string and integer options may define choices".into());
            }
            if choices.is_empty() || choices.len() > 25 {
                return Err("Command option choices must contain 1-25 entries".into());
            }
            let mut choice_values = std::collections::HashSet::new();
            for choice in choices {
                if choice.name.is_empty()
                    || choice.name.len() > 100
                    || choice.value.is_empty()
                    || choice.value.len() > 100
                    || !choice_values.insert(choice.value.as_str())
                {
                    return Err("Command option choices must have unique bounded values".into());
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_slash_command_arguments(
    options: &[SlashCommandOption],
    value: &serde_json::Value,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("Command arguments must be a JSON object")?;
    if object
        .keys()
        .any(|name| !options.iter().any(|option| option.name == *name))
    {
        return Err("Command arguments contain an unknown option".into());
    }
    for option in options {
        let Some(argument) = object.get(&option.name) else {
            if option.required {
                return Err(format!("Missing required command option: {}", option.name));
            }
            continue;
        };
        let type_matches = match option.option_type.as_str() {
            "string" | "user" | "channel" | "role" => argument.is_string(),
            "integer" => argument.as_i64().is_some(),
            "boolean" => argument.is_boolean(),
            _ => false,
        };
        if !type_matches {
            return Err(format!("Invalid value for command option: {}", option.name));
        }
        if let Some(choices) = &option.choices {
            let candidate = argument
                .as_str()
                .map(str::to_owned)
                .or_else(|| argument.as_i64().map(|number| number.to_string()))
                .ok_or_else(|| format!("Invalid value for command option: {}", option.name))?;
            if !choices.iter().any(|choice| choice.value == candidate) {
                return Err(format!(
                    "Invalid choice for command option: {}",
                    option.name
                ));
            }
        }
    }
    Ok(())
}
