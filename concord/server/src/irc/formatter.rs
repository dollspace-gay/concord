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
