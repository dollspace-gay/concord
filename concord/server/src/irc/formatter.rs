use super::numerics::*;
use super::parser::IrcMessage;

/// Helper to build IRC reply lines. All functions return formatted strings
/// ready to send (caller appends \r\n).
const SERVER_NAME: &str = "concord";

pub fn server_name() -> &'static str {
    SERVER_NAME
}

/// :concord 001 nick :Welcome to Concord, nick!
pub fn rpl_welcome(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_WELCOME,
        vec![nick.into(), format!("Welcome to Concord, {}!", nick)],
    )
    .format()
}

/// :concord 002 nick :Your host is concord, running version 0.1.0
pub fn rpl_yourhost(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_YOURHOST,
        vec![
            nick.into(),
            format!("Your host is {}, running version 0.1.0", SERVER_NAME),
        ],
    )
    .format()
}

/// :concord 003 nick :This server was created ...
pub fn rpl_created(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_CREATED,
        vec![nick.into(), "This server was created today".into()],
    )
    .format()
}

/// :concord 004 nick concord 0.1.0 o o
pub fn rpl_myinfo(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_MYINFO,
        vec![
            nick.into(),
            SERVER_NAME.into(),
            "0.1.0".into(),
            "o".into(),
            "o".into(),
        ],
    )
    .format()
}

/// :concord 422 nick :MOTD File is missing
pub fn err_nomotd(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NOMOTD,
        vec![nick.into(), "MOTD File is missing".into()],
    )
    .format()
}

/// :nick!nick@concord JOIN #channel
pub fn join(nick: &str, channel: &str) -> String {
    IrcMessage {
        prefix: Some(format!("{}!{}@{}", nick, nick, SERVER_NAME)),
        command: "JOIN".into(),
        params: vec![channel.into()],
    }
    .format()
}

/// :nick!nick@concord PART #channel [:reason]
pub fn part(nick: &str, channel: &str, reason: Option<&str>) -> String {
    let mut params = vec![channel.to_string()];
    if let Some(r) = reason {
        params.push(r.to_string());
    }
    IrcMessage {
        prefix: Some(format!("{}!{}@{}", nick, nick, SERVER_NAME)),
        command: "PART".into(),
        params,
    }
    .format()
}

/// :nick!nick@concord PRIVMSG target :message
pub fn privmsg(nick: &str, target: &str, message: &str) -> String {
    IrcMessage {
        prefix: Some(format!("{}!{}@{}", nick, nick, SERVER_NAME)),
        command: "PRIVMSG".into(),
        params: vec![target.into(), message.into()],
    }
    .format()
}

/// :nick!nick@concord QUIT [:reason]
pub fn quit(nick: &str, reason: Option<&str>) -> String {
    let mut params = Vec::new();
    if let Some(r) = reason {
        params.push(r.to_string());
    }
    IrcMessage {
        prefix: Some(format!("{}!{}@{}", nick, nick, SERVER_NAME)),
        command: "QUIT".into(),
        params,
    }
    .format()
}

/// :nick!nick@concord NICK newnick
pub fn nick_change(old_nick: &str, new_nick: &str) -> String {
    IrcMessage {
        prefix: Some(format!("{}!{}@{}", old_nick, old_nick, SERVER_NAME)),
        command: "NICK".into(),
        params: vec![new_nick.into()],
    }
    .format()
}

/// :concord 332 nick #channel :topic text
pub fn rpl_topic(nick: &str, channel: &str, topic: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_TOPIC,
        vec![nick.into(), channel.into(), topic.into()],
    )
    .format()
}

/// :concord 331 nick #channel :No topic is set
pub fn rpl_notopic(nick: &str, channel: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_NOTOPIC,
        vec![nick.into(), channel.into(), "No topic is set".into()],
    )
    .format()
}

/// :nick!nick@concord TOPIC #channel :new topic
pub fn topic_change(nick: &str, channel: &str, topic: &str) -> String {
    IrcMessage {
        prefix: Some(format!("{}!{}@{}", nick, nick, SERVER_NAME)),
        command: "TOPIC".into(),
        params: vec![channel.into(), topic.into()],
    }
    .format()
}

/// :concord 353 nick = #channel :nick1 nick2 nick3
pub fn rpl_namreply(nick: &str, channel: &str, members: &[String]) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_NAMREPLY,
        vec![nick.into(), "=".into(), channel.into(), members.join(" ")],
    )
    .format()
}

/// :concord 366 nick #channel :End of /NAMES list
pub fn rpl_endofnames(nick: &str, channel: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_ENDOFNAMES,
        vec![nick.into(), channel.into(), "End of /NAMES list".into()],
    )
    .format()
}

/// :concord 322 nick #channel member_count :topic
pub fn rpl_list(nick: &str, channel: &str, member_count: usize, topic: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_LIST,
        vec![
            nick.into(),
            channel.into(),
            member_count.to_string(),
            topic.into(),
        ],
    )
    .format()
}

/// :concord 323 nick :End of /LIST
pub fn rpl_listend(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_LISTEND,
        vec![nick.into(), "End of /LIST".into()],
    )
    .format()
}

/// :concord 311 requestor nick user host * :realname
pub fn rpl_whoisuser(requestor: &str, nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_WHOISUSER,
        vec![
            requestor.into(),
            nick.into(),
            nick.into(),
            SERVER_NAME.into(),
            "*".into(),
            nick.into(),
        ],
    )
    .format()
}

/// :concord 312 requestor nick server :server info
pub fn rpl_whoisserver(requestor: &str, nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_WHOISSERVER,
        vec![
            requestor.into(),
            nick.into(),
            SERVER_NAME.into(),
            "Concord IRC-compatible chat server".into(),
        ],
    )
    .format()
}

/// :concord 318 requestor nick :End of /WHOIS list
pub fn rpl_endofwhois(requestor: &str, nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        RPL_ENDOFWHOIS,
        vec![requestor.into(), nick.into(), "End of /WHOIS list".into()],
    )
    .format()
}

// Error replies

/// :concord 401 nick target :No such nick/channel
pub fn err_nosuchnick(nick: &str, target: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NOSUCHNICK,
        vec![nick.into(), target.into(), "No such nick/channel".into()],
    )
    .format()
}

/// :concord 403 nick channel :No such channel
pub fn err_nosuchchannel(nick: &str, channel: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NOSUCHCHANNEL,
        vec![nick.into(), channel.into(), "No such channel".into()],
    )
    .format()
}

/// :concord 421 nick command :Unknown command
pub fn err_unknowncommand(nick: &str, command: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_UNKNOWNCOMMAND,
        vec![nick.into(), command.into(), "Unknown command".into()],
    )
    .format()
}

/// :concord 431 nick :No nickname given
pub fn err_nonicknamegiven(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NONICKNAMEGIVEN,
        vec![nick.into(), "No nickname given".into()],
    )
    .format()
}

/// :concord 433 nick newnick :Nickname is already in use
pub fn err_nicknameinuse(nick: &str, wanted: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NICKNAMEINUSE,
        vec![
            nick.into(),
            wanted.into(),
            "Nickname is already in use".into(),
        ],
    )
    .format()
}

/// :concord 442 nick channel :You're not on that channel
pub fn err_notonchannel(nick: &str, channel: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NOTONCHANNEL,
        vec![
            nick.into(),
            channel.into(),
            "You're not on that channel".into(),
        ],
    )
    .format()
}

/// :concord 451 * :You have not registered
pub fn err_notregistered() -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NOTREGISTERED,
        vec!["*".into(), "You have not registered".into()],
    )
    .format()
}

/// :concord 461 nick command :Not enough parameters
pub fn err_needmoreparams(nick: &str, command: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_NEEDMOREPARAMS,
        vec![nick.into(), command.into(), "Not enough parameters".into()],
    )
    .format()
}

/// :concord 462 nick :You may not reregister
pub fn err_alreadyregistered(nick: &str) -> String {
    IrcMessage::server_reply(
        SERVER_NAME,
        ERR_ALREADYREGISTERED,
        vec![nick.into(), "You may not reregister".into()],
    )
    .format()
}

/// PING :token
pub fn ping(token: &str) -> String {
    IrcMessage {
        prefix: None,
        command: "PING".into(),
        params: vec![token.into()],
    }
    .format()
}

/// :concord PONG concord :token
pub fn pong(token: &str) -> String {
    IrcMessage {
        prefix: Some(SERVER_NAME.into()),
        command: "PONG".into(),
        params: vec![SERVER_NAME.into(), token.into()],
    }
    .format()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Server name helper ──

    #[test]
    fn test_server_name() {
        assert_eq!(server_name(), "concord");
    }

    // ── Welcome burst (001-004) ──

    #[test]
    fn test_rpl_welcome() {
        let result = rpl_welcome("alice");
        assert_eq!(result, ":concord 001 alice :Welcome to Concord, alice!");
    }

    #[test]
    fn test_rpl_yourhost() {
        let result = rpl_yourhost("alice");
        assert_eq!(
            result,
            ":concord 002 alice :Your host is concord, running version 0.1.0"
        );
    }

    #[test]
    fn test_rpl_created() {
        let result = rpl_created("alice");
        assert_eq!(result, ":concord 003 alice :This server was created today");
    }

    #[test]
    fn test_rpl_myinfo() {
        let result = rpl_myinfo("alice");
        assert_eq!(result, ":concord 004 alice concord 0.1.0 o o");
    }

    // ── MOTD ──

    #[test]
    fn test_err_nomotd() {
        let result = err_nomotd("alice");
        assert_eq!(result, ":concord 422 alice :MOTD File is missing");
    }

    // ── JOIN / PART / QUIT / NICK ──

    #[test]
    fn test_join_format() {
        let result = join("alice", "#general");
        assert_eq!(result, ":alice!alice@concord JOIN #general");
    }

    #[test]
    fn test_part_without_reason() {
        let result = part("alice", "#general", None);
        assert_eq!(result, ":alice!alice@concord PART #general");
    }

    #[test]
    fn test_part_with_reason() {
        let result = part("alice", "#general", Some("Goodbye"));
        // Single-word reason has no colon prefix (format() only adds : for spaces/empty)
        assert_eq!(result, ":alice!alice@concord PART #general Goodbye");
    }

    #[test]
    fn test_privmsg_format() {
        let result = privmsg("alice", "#general", "Hello world");
        assert_eq!(result, ":alice!alice@concord PRIVMSG #general :Hello world");
    }

    #[test]
    fn test_privmsg_single_word() {
        let result = privmsg("alice", "bob", "hello");
        // Single word trailing should NOT get colon since it has no spaces
        // But the format method adds colon only for last param with spaces or empty
        // "hello" has no spaces, so no colon prefix
        assert_eq!(result, ":alice!alice@concord PRIVMSG bob hello");
    }

    #[test]
    fn test_quit_without_reason() {
        let result = quit("alice", None);
        assert_eq!(result, ":alice!alice@concord QUIT");
    }

    #[test]
    fn test_quit_with_reason() {
        let result = quit("alice", Some("Gone to lunch"));
        assert_eq!(result, ":alice!alice@concord QUIT :Gone to lunch");
    }

    #[test]
    fn test_nick_change() {
        let result = nick_change("alice", "alice_away");
        assert_eq!(result, ":alice!alice@concord NICK alice_away");
    }

    // ── TOPIC ──

    #[test]
    fn test_rpl_topic() {
        let result = rpl_topic("alice", "#general", "Welcome to the server!");
        assert_eq!(
            result,
            ":concord 332 alice #general :Welcome to the server!"
        );
    }

    #[test]
    fn test_rpl_notopic() {
        let result = rpl_notopic("alice", "#general");
        assert_eq!(result, ":concord 331 alice #general :No topic is set");
    }

    #[test]
    fn test_topic_change_format() {
        let result = topic_change("alice", "#general", "New topic here");
        assert_eq!(
            result,
            ":alice!alice@concord TOPIC #general :New topic here"
        );
    }

    // ── NAMES ──

    #[test]
    fn test_rpl_namreply() {
        let members = vec![
            "alice".to_string(),
            "bob".to_string(),
            "charlie".to_string(),
        ];
        let result = rpl_namreply("alice", "#general", &members);
        assert_eq!(result, ":concord 353 alice = #general :alice bob charlie");
    }

    #[test]
    fn test_rpl_namreply_single_member() {
        let members = vec!["alice".to_string()];
        let result = rpl_namreply("alice", "#general", &members);
        assert_eq!(result, ":concord 353 alice = #general alice");
    }

    #[test]
    fn test_rpl_endofnames() {
        let result = rpl_endofnames("alice", "#general");
        assert_eq!(result, ":concord 366 alice #general :End of /NAMES list");
    }

    // ── LIST ──

    #[test]
    fn test_rpl_list() {
        let result = rpl_list("alice", "#general", 42, "Welcome channel");
        assert_eq!(result, ":concord 322 alice #general 42 :Welcome channel");
    }

    #[test]
    fn test_rpl_list_empty_topic() {
        let result = rpl_list("alice", "#general", 5, "");
        assert_eq!(result, ":concord 322 alice #general 5 :");
    }

    #[test]
    fn test_rpl_listend() {
        let result = rpl_listend("alice");
        assert_eq!(result, ":concord 323 alice :End of /LIST");
    }

    // ── WHOIS ──

    #[test]
    fn test_rpl_whoisuser() {
        let result = rpl_whoisuser("alice", "bob");
        assert_eq!(result, ":concord 311 alice bob bob concord * bob");
    }

    #[test]
    fn test_rpl_whoisserver() {
        let result = rpl_whoisserver("alice", "bob");
        assert_eq!(
            result,
            ":concord 312 alice bob concord :Concord IRC-compatible chat server"
        );
    }

    #[test]
    fn test_rpl_endofwhois() {
        let result = rpl_endofwhois("alice", "bob");
        assert_eq!(result, ":concord 318 alice bob :End of /WHOIS list");
    }

    // ── Error replies ──

    #[test]
    fn test_err_nosuchnick() {
        let result = err_nosuchnick("alice", "nobody");
        assert_eq!(result, ":concord 401 alice nobody :No such nick/channel");
    }

    #[test]
    fn test_err_nosuchchannel() {
        let result = err_nosuchchannel("alice", "#nonexistent");
        assert_eq!(result, ":concord 403 alice #nonexistent :No such channel");
    }

    #[test]
    fn test_err_unknowncommand() {
        let result = err_unknowncommand("alice", "FOOBAR");
        assert_eq!(result, ":concord 421 alice FOOBAR :Unknown command");
    }

    #[test]
    fn test_err_nonicknamegiven() {
        let result = err_nonicknamegiven("alice");
        assert_eq!(result, ":concord 431 alice :No nickname given");
    }

    #[test]
    fn test_err_nicknameinuse() {
        let result = err_nicknameinuse("*", "alice");
        assert_eq!(result, ":concord 433 * alice :Nickname is already in use");
    }

    #[test]
    fn test_err_notonchannel() {
        let result = err_notonchannel("alice", "#secret");
        assert_eq!(
            result,
            ":concord 442 alice #secret :You're not on that channel"
        );
    }

    #[test]
    fn test_err_notregistered() {
        let result = err_notregistered();
        assert_eq!(result, ":concord 451 * :You have not registered");
    }

    #[test]
    fn test_err_needmoreparams() {
        let result = err_needmoreparams("alice", "JOIN");
        assert_eq!(result, ":concord 461 alice JOIN :Not enough parameters");
    }

    #[test]
    fn test_err_alreadyregistered() {
        let result = err_alreadyregistered("alice");
        assert_eq!(result, ":concord 462 alice :You may not reregister");
    }

    // ── PING / PONG ──

    #[test]
    fn test_ping_format() {
        let result = ping("token123");
        assert_eq!(result, "PING token123");
    }

    #[test]
    fn test_pong_format() {
        let result = pong("token123");
        assert_eq!(result, ":concord PONG concord token123");
    }

    #[test]
    fn test_pong_with_spaces() {
        let result = pong("my token value");
        assert_eq!(result, ":concord PONG concord :my token value");
    }

    // ── Prefix formatting consistency ──

    #[test]
    fn test_prefix_format_nick_user_host() {
        let result = join("testuser", "#test");
        // Should have :nick!nick@concord prefix
        assert!(result.starts_with(":testuser!testuser@concord "));
    }

    #[test]
    fn test_all_user_commands_use_same_prefix_format() {
        let j = join("user1", "#test");
        let p = part("user1", "#test", None);
        let q = quit("user1", None);
        let n = nick_change("user1", "user2");
        let m = privmsg("user1", "#test", "hi");
        let t = topic_change("user1", "#test", "topic");

        let prefix = ":user1!user1@concord ";
        assert!(j.starts_with(prefix), "JOIN prefix mismatch: {}", j);
        assert!(p.starts_with(prefix), "PART prefix mismatch: {}", p);
        assert!(q.starts_with(prefix), "QUIT prefix mismatch: {}", q);
        assert!(n.starts_with(prefix), "NICK prefix mismatch: {}", n);
        assert!(m.starts_with(prefix), "PRIVMSG prefix mismatch: {}", m);
        assert!(t.starts_with(prefix), "TOPIC prefix mismatch: {}", t);
    }

    #[test]
    fn test_all_server_replies_use_concord_prefix() {
        let replies = vec![
            rpl_welcome("u"),
            rpl_yourhost("u"),
            rpl_created("u"),
            rpl_myinfo("u"),
            err_nomotd("u"),
            rpl_topic("u", "#c", "t"),
            rpl_notopic("u", "#c"),
            rpl_namreply("u", "#c", &["a".into()]),
            rpl_endofnames("u", "#c"),
            rpl_listend("u"),
            err_nosuchnick("u", "t"),
            err_nosuchchannel("u", "#c"),
            err_unknowncommand("u", "X"),
            err_nonicknamegiven("u"),
            err_nicknameinuse("u", "n"),
            err_notonchannel("u", "#c"),
            err_notregistered(),
            err_needmoreparams("u", "CMD"),
            err_alreadyregistered("u"),
        ];

        for reply in &replies {
            assert!(
                reply.starts_with(":concord "),
                "Server reply missing :concord prefix: {}",
                reply
            );
        }
    }
}
