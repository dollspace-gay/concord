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
mod tests;
