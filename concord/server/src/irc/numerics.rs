/// IRC numeric reply codes per RFC 2812.
// Connection registration
pub const RPL_WELCOME: &str = "001";
pub const RPL_YOURHOST: &str = "002";
pub const RPL_CREATED: &str = "003";
pub const RPL_MYINFO: &str = "004";

// Channel operations
pub const RPL_TOPIC: &str = "332";
pub const RPL_NOTOPIC: &str = "331";
pub const RPL_NAMREPLY: &str = "353";
pub const RPL_ENDOFNAMES: &str = "366";

// LIST
pub const RPL_LIST: &str = "322";
pub const RPL_LISTEND: &str = "323";

// WHO / WHOIS
pub const RPL_WHOREPLY: &str = "352";
pub const RPL_ENDOFWHO: &str = "315";
pub const RPL_WHOISUSER: &str = "311";
pub const RPL_WHOISSERVER: &str = "312";
pub const RPL_ENDOFWHOIS: &str = "318";
pub const RPL_WHOISCHANNELS: &str = "319";

// MOTD
pub const RPL_MOTDSTART: &str = "375";
pub const RPL_MOTD: &str = "372";
pub const RPL_ENDOFMOTD: &str = "376";
pub const ERR_NOMOTD: &str = "422";

// Errors
pub const ERR_NOSUCHNICK: &str = "401";
pub const ERR_NOSUCHCHANNEL: &str = "403";
pub const ERR_CANNOTSENDTOCHAN: &str = "404";
pub const ERR_UNKNOWNCOMMAND: &str = "421";
pub const ERR_NONICKNAMEGIVEN: &str = "431";
pub const ERR_NICKNAMEINUSE: &str = "433";
pub const ERR_NOTONCHANNEL: &str = "442";
pub const ERR_NOTREGISTERED: &str = "451";
pub const ERR_NEEDMOREPARAMS: &str = "461";
pub const ERR_ALREADYREGISTERED: &str = "462";
pub const ERR_PASSWDMISMATCH: &str = "464";
