/// An IRC protocol message per RFC 2812.
///
/// Wire format: `[:prefix] COMMAND [params...] [:trailing]\r\n`
///
/// Examples:
///   `:nick!user@host PRIVMSG #channel :Hello world\r\n`
///   `NICK alice\r\n`
///   `JOIN #general\r\n`
#[derive(Debug, Clone, PartialEq)]
pub struct IrcMessage {
    pub prefix: Option<String>,
    pub command: String,
    pub params: Vec<String>,
}

impl IrcMessage {
    /// Parse a single IRC line (without the trailing \r\n).
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        let line = line.trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            return Err(ParseError::Empty);
        }

        let mut remaining = line;
        let mut prefix = None;

        // Parse optional prefix
        if remaining.starts_with(':') {
            remaining = &remaining[1..];
            match remaining.find(' ') {
                Some(idx) => {
                    prefix = Some(remaining[..idx].to_string());
                    remaining = remaining[idx..].trim_start();
                }
                None => return Err(ParseError::MissingCommand),
            }
        }

        // Parse command
        let command;
        match remaining.find(' ') {
            Some(idx) => {
                command = remaining[..idx].to_uppercase();
                remaining = remaining[idx..].trim_start();
            }
            None => {
                command = remaining.to_uppercase();
                remaining = "";
            }
        }

        if command.is_empty() {
            return Err(ParseError::MissingCommand);
        }

        // Parse parameters
        let mut params = Vec::new();
        while !remaining.is_empty() {
            if let Some(trailing) = remaining.strip_prefix(':') {
                // Trailing parameter — everything after the colon
                params.push(trailing.to_string());
                break;
            }

            match remaining.find(' ') {
                Some(idx) => {
                    params.push(remaining[..idx].to_string());
                    remaining = remaining[idx..].trim_start();
                }
                None => {
                    params.push(remaining.to_string());
                    break;
                }
            }
        }

        Ok(IrcMessage {
            prefix,
            command,
            params,
        })
    }

    /// Format this message back to IRC wire format (without trailing \r\n).
    pub fn format(&self) -> String {
        let mut out = String::with_capacity(512);

        if let Some(ref prefix) = self.prefix {
            out.push(':');
            out.push_str(prefix);
            out.push(' ');
        }

        out.push_str(&self.command);

        for (i, param) in self.params.iter().enumerate() {
            out.push(' ');
            // Last param gets colon prefix if it contains spaces or is empty
            if i == self.params.len() - 1 && (param.contains(' ') || param.is_empty()) {
                out.push(':');
            }
            // Strip \r\n to prevent IRC command injection via user content
            let sanitized = param.replace(['\r', '\n'], " ");
            out.push_str(&sanitized);
        }

        out
    }

    /// Create a server reply with the given prefix.
    pub fn server_reply(server_name: &str, command: &str, params: Vec<String>) -> Self {
        IrcMessage {
            prefix: Some(server_name.to_string()),
            command: command.to_string(),
            params,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Empty,
    MissingCommand,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty message"),
            ParseError::MissingCommand => write!(f, "missing command"),
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_command() {
        let msg = IrcMessage::parse("NICK alice").unwrap();
        assert_eq!(msg.prefix, None);
        assert_eq!(msg.command, "NICK");
        assert_eq!(msg.params, vec!["alice"]);
    }

    #[test]
    fn test_parse_with_prefix() {
        let msg = IrcMessage::parse(":alice!alice@host PRIVMSG #general :Hello world").unwrap();
        assert_eq!(msg.prefix, Some("alice!alice@host".into()));
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["#general", "Hello world"]);
    }

    #[test]
    fn test_parse_join() {
        let msg = IrcMessage::parse("JOIN #general").unwrap();
        assert_eq!(msg.command, "JOIN");
        assert_eq!(msg.params, vec!["#general"]);
    }

    #[test]
    fn test_parse_no_params() {
        let msg = IrcMessage::parse("QUIT").unwrap();
        assert_eq!(msg.command, "QUIT");
        assert!(msg.params.is_empty());
    }

    #[test]
    fn test_parse_quit_with_reason() {
        let msg = IrcMessage::parse("QUIT :Gone to lunch").unwrap();
        assert_eq!(msg.command, "QUIT");
        assert_eq!(msg.params, vec!["Gone to lunch"]);
    }

    #[test]
    fn test_parse_user_command() {
        let msg = IrcMessage::parse("USER alice 0 * :Alice Smith").unwrap();
        assert_eq!(msg.command, "USER");
        assert_eq!(msg.params, vec!["alice", "0", "*", "Alice Smith"]);
    }

    #[test]
    fn test_parse_strips_crlf() {
        let msg = IrcMessage::parse("NICK alice\r\n").unwrap();
        assert_eq!(msg.command, "NICK");
        assert_eq!(msg.params, vec!["alice"]);
    }

    #[test]
    fn test_parse_command_case_insensitive() {
        let msg = IrcMessage::parse("privmsg #test :hello").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
    }

    #[test]
    fn test_parse_empty() {
        assert_eq!(IrcMessage::parse(""), Err(ParseError::Empty));
    }

    #[test]
    fn test_parse_prefix_only() {
        assert_eq!(
            IrcMessage::parse(":prefix"),
            Err(ParseError::MissingCommand)
        );
    }

    #[test]
    fn test_format_simple() {
        let msg = IrcMessage {
            prefix: None,
            command: "NICK".into(),
            params: vec!["alice".into()],
        };
        assert_eq!(msg.format(), "NICK alice");
    }

    #[test]
    fn test_format_with_prefix_and_trailing() {
        let msg = IrcMessage {
            prefix: Some("server".into()),
            command: "PRIVMSG".into(),
            params: vec!["#general".into(), "Hello world".into()],
        };
        assert_eq!(msg.format(), ":server PRIVMSG #general :Hello world");
    }

    #[test]
    fn test_format_numeric() {
        let msg = IrcMessage {
            prefix: Some("concord".into()),
            command: "001".into(),
            params: vec!["alice".into(), "Welcome to Concord!".into()],
        };
        assert_eq!(msg.format(), ":concord 001 alice :Welcome to Concord!");
    }

    #[test]
    fn test_roundtrip() {
        let original = ":server PRIVMSG #channel :Hello world";
        let msg = IrcMessage::parse(original).unwrap();
        assert_eq!(msg.format(), original);
    }

    #[test]
    fn test_pass_command() {
        let msg = IrcMessage::parse("PASS secrettoken123").unwrap();
        assert_eq!(msg.command, "PASS");
        assert_eq!(msg.params, vec!["secrettoken123"]);
    }

    #[test]
    fn test_mode_command() {
        let msg = IrcMessage::parse("MODE #channel +o alice").unwrap();
        assert_eq!(msg.command, "MODE");
        assert_eq!(msg.params, vec!["#channel", "+o", "alice"]);
    }

    // ── Additional IRC command type tests ──

    #[test]
    fn test_parse_part_with_reason() {
        let msg = IrcMessage::parse("PART #general :Leaving for now").unwrap();
        assert_eq!(msg.command, "PART");
        assert_eq!(msg.params, vec!["#general", "Leaving for now"]);
    }

    #[test]
    fn test_parse_part_no_reason() {
        let msg = IrcMessage::parse("PART #general").unwrap();
        assert_eq!(msg.command, "PART");
        assert_eq!(msg.params, vec!["#general"]);
    }

    #[test]
    fn test_parse_topic_set() {
        let msg = IrcMessage::parse("TOPIC #general :Welcome to the general channel").unwrap();
        assert_eq!(msg.command, "TOPIC");
        assert_eq!(
            msg.params,
            vec!["#general", "Welcome to the general channel"]
        );
    }

    #[test]
    fn test_parse_topic_query() {
        let msg = IrcMessage::parse("TOPIC #general").unwrap();
        assert_eq!(msg.command, "TOPIC");
        assert_eq!(msg.params, vec!["#general"]);
    }

    #[test]
    fn test_parse_names_command() {
        let msg = IrcMessage::parse("NAMES #general").unwrap();
        assert_eq!(msg.command, "NAMES");
        assert_eq!(msg.params, vec!["#general"]);
    }

    #[test]
    fn test_parse_list_no_params() {
        let msg = IrcMessage::parse("LIST").unwrap();
        assert_eq!(msg.command, "LIST");
        assert!(msg.params.is_empty());
    }

    #[test]
    fn test_parse_who_command() {
        let msg = IrcMessage::parse("WHO #general").unwrap();
        assert_eq!(msg.command, "WHO");
        assert_eq!(msg.params, vec!["#general"]);
    }

    #[test]
    fn test_parse_whois_command() {
        let msg = IrcMessage::parse("WHOIS alice").unwrap();
        assert_eq!(msg.command, "WHOIS");
        assert_eq!(msg.params, vec!["alice"]);
    }

    #[test]
    fn test_parse_kick_command() {
        let msg = IrcMessage::parse("KICK #general baduser :Spamming").unwrap();
        assert_eq!(msg.command, "KICK");
        assert_eq!(msg.params, vec!["#general", "baduser", "Spamming"]);
    }

    #[test]
    fn test_parse_kick_no_reason() {
        let msg = IrcMessage::parse("KICK #general baduser").unwrap();
        assert_eq!(msg.command, "KICK");
        assert_eq!(msg.params, vec!["#general", "baduser"]);
    }

    #[test]
    fn test_parse_notice_command() {
        let msg = IrcMessage::parse("NOTICE alice :You have been warned").unwrap();
        assert_eq!(msg.command, "NOTICE");
        assert_eq!(msg.params, vec!["alice", "You have been warned"]);
    }

    #[test]
    fn test_parse_cap_ls() {
        let msg = IrcMessage::parse("CAP LS 302").unwrap();
        assert_eq!(msg.command, "CAP");
        assert_eq!(msg.params, vec!["LS", "302"]);
    }

    #[test]
    fn test_parse_cap_end() {
        let msg = IrcMessage::parse("CAP END").unwrap();
        assert_eq!(msg.command, "CAP");
        assert_eq!(msg.params, vec!["END"]);
    }

    #[test]
    fn test_parse_authenticate() {
        let msg = IrcMessage::parse("AUTHENTICATE PLAIN").unwrap();
        assert_eq!(msg.command, "AUTHENTICATE");
        assert_eq!(msg.params, vec!["PLAIN"]);
    }

    #[test]
    fn test_parse_ping() {
        let msg = IrcMessage::parse("PING :some-token-123").unwrap();
        assert_eq!(msg.command, "PING");
        assert_eq!(msg.params, vec!["some-token-123"]);
    }

    #[test]
    fn test_parse_pong() {
        let msg = IrcMessage::parse("PONG :some-token-123").unwrap();
        assert_eq!(msg.command, "PONG");
        assert_eq!(msg.params, vec!["some-token-123"]);
    }

    #[test]
    fn test_parse_privmsg_channel() {
        let msg = IrcMessage::parse("PRIVMSG #general :Hello everyone!").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["#general", "Hello everyone!"]);
    }

    #[test]
    fn test_parse_privmsg_dm() {
        let msg = IrcMessage::parse("PRIVMSG bob :Hey, what's up?").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["bob", "Hey, what's up?"]);
    }

    // ── Edge cases ──

    #[test]
    fn test_parse_unicode_in_trailing() {
        let msg = IrcMessage::parse("PRIVMSG #general :Hello \u{1f600} \u{1f44d} world").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(
            msg.params,
            vec!["#general", "Hello \u{1f600} \u{1f44d} world"]
        );
    }

    #[test]
    fn test_parse_unicode_in_nick() {
        let msg = IrcMessage::parse("NICK \u{00e9}milie").unwrap();
        assert_eq!(msg.command, "NICK");
        assert_eq!(msg.params, vec!["\u{00e9}milie"]);
    }

    #[test]
    fn test_parse_empty_trailing_param() {
        let msg = IrcMessage::parse("PRIVMSG #general :").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["#general", ""]);
    }

    #[test]
    fn test_parse_multiple_colons_in_trailing() {
        let msg =
            IrcMessage::parse("PRIVMSG #general :time is 12:30:45 and url is http://example.com")
                .unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(
            msg.params,
            vec!["#general", "time is 12:30:45 and url is http://example.com"]
        );
    }

    #[test]
    fn test_parse_extra_spaces_between_params() {
        let msg = IrcMessage::parse("MODE   #channel   +o   alice").unwrap();
        assert_eq!(msg.command, "MODE");
        assert_eq!(msg.params, vec!["#channel", "+o", "alice"]);
    }

    #[test]
    fn test_parse_only_crlf() {
        assert_eq!(IrcMessage::parse("\r\n"), Err(ParseError::Empty));
    }

    #[test]
    fn test_parse_only_lf() {
        assert_eq!(IrcMessage::parse("\n"), Err(ParseError::Empty));
    }

    #[test]
    fn test_parse_only_cr() {
        assert_eq!(IrcMessage::parse("\r"), Err(ParseError::Empty));
    }

    #[test]
    fn test_parse_command_case_variations() {
        for cmd in &["nick", "Nick", "NICK", "nICk"] {
            let msg = IrcMessage::parse(&format!("{} alice", cmd)).unwrap();
            assert_eq!(msg.command, "NICK", "Failed for input: {}", cmd);
        }
    }

    #[test]
    fn test_parse_ctcp_action() {
        let msg = IrcMessage::parse("PRIVMSG #general :\x01ACTION dances around\x01").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params[1], "\x01ACTION dances around\x01");
    }

    #[test]
    fn test_parse_ctcp_version() {
        let msg = IrcMessage::parse("PRIVMSG alice :\x01VERSION\x01").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params[1], "\x01VERSION\x01");
    }

    #[test]
    fn test_parse_very_long_line() {
        let long_msg = format!("PRIVMSG #general :{}", "A".repeat(600));
        let msg = IrcMessage::parse(&long_msg).unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params[1].len(), 600);
    }

    #[test]
    fn test_parse_numeric_reply() {
        let msg = IrcMessage::parse(":concord 001 alice :Welcome to Concord!").unwrap();
        assert_eq!(msg.prefix, Some("concord".into()));
        assert_eq!(msg.command, "001");
        assert_eq!(msg.params, vec!["alice", "Welcome to Concord!"]);
    }

    #[test]
    fn test_parse_numeric_353_namreply() {
        let msg = IrcMessage::parse(":concord 353 alice = #general :alice bob charlie").unwrap();
        assert_eq!(msg.command, "353");
        assert_eq!(
            msg.params,
            vec!["alice", "=", "#general", "alice bob charlie"]
        );
    }

    #[test]
    fn test_parse_numeric_433_nick_in_use() {
        let msg = IrcMessage::parse(":concord 433 * alice :Nickname is already in use").unwrap();
        assert_eq!(msg.command, "433");
        assert_eq!(msg.params, vec!["*", "alice", "Nickname is already in use"]);
    }

    #[test]
    fn test_parse_join_multiple_channels() {
        let msg = IrcMessage::parse("JOIN #general,#random,#dev").unwrap();
        assert_eq!(msg.command, "JOIN");
        assert_eq!(msg.params, vec!["#general,#random,#dev"]);
    }

    #[test]
    fn test_parse_prefix_with_full_hostmask() {
        let msg =
            IrcMessage::parse(":nick!~user@some.host.example.com PRIVMSG #test :hello").unwrap();
        assert_eq!(msg.prefix, Some("nick!~user@some.host.example.com".into()));
        assert_eq!(msg.command, "PRIVMSG");
    }

    #[test]
    fn test_parse_server_prefix() {
        let msg = IrcMessage::parse(":irc.example.com NOTICE * :Server restarting").unwrap();
        assert_eq!(msg.prefix, Some("irc.example.com".into()));
        assert_eq!(msg.command, "NOTICE");
        assert_eq!(msg.params, vec!["*", "Server restarting"]);
    }

    #[test]
    fn test_format_empty_last_param_gets_colon() {
        let msg = IrcMessage {
            prefix: None,
            command: "PRIVMSG".into(),
            params: vec!["#test".into(), "".into()],
        };
        assert_eq!(msg.format(), "PRIVMSG #test :");
    }

    #[test]
    fn test_format_no_params() {
        let msg = IrcMessage {
            prefix: None,
            command: "QUIT".into(),
            params: vec![],
        };
        assert_eq!(msg.format(), "QUIT");
    }

    #[test]
    fn test_format_single_param_no_spaces() {
        let msg = IrcMessage {
            prefix: None,
            command: "NICK".into(),
            params: vec!["alice".into()],
        };
        // Last param without spaces should NOT get a colon prefix
        assert_eq!(msg.format(), "NICK alice");
    }

    #[test]
    fn test_server_reply_constructor() {
        let msg =
            IrcMessage::server_reply("concord", "001", vec!["alice".into(), "Welcome!".into()]);
        assert_eq!(msg.prefix, Some("concord".into()));
        assert_eq!(msg.command, "001");
        assert_eq!(msg.params, vec!["alice", "Welcome!"]);
    }

    #[test]
    fn test_parse_error_display() {
        assert_eq!(format!("{}", ParseError::Empty), "empty message");
        assert_eq!(format!("{}", ParseError::MissingCommand), "missing command");
    }

    #[test]
    fn test_parse_prefix_with_space_before_command() {
        let msg = IrcMessage::parse(":prefix  NICK alice").unwrap();
        assert_eq!(msg.prefix, Some("prefix".into()));
        assert_eq!(msg.command, "NICK");
        assert_eq!(msg.params, vec!["alice"]);
    }

    #[test]
    fn test_roundtrip_quit_with_reason() {
        let original = ":alice!alice@concord QUIT :Gone to lunch";
        let msg = IrcMessage::parse(original).unwrap();
        assert_eq!(msg.format(), original);
    }

    #[test]
    fn test_roundtrip_join() {
        let original = ":alice!alice@concord JOIN #general";
        let msg = IrcMessage::parse(original).unwrap();
        assert_eq!(msg.format(), original);
    }

    #[test]
    fn test_roundtrip_nick() {
        let original = "NICK alice";
        let msg = IrcMessage::parse(original).unwrap();
        assert_eq!(msg.format(), original);
    }
}
