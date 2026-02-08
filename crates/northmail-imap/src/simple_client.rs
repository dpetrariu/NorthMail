//! Simple IMAP client using raw protocol (no async-imap)
//!
//! This client is designed to work reliably in any async context.

use async_native_tls::TlsConnector;
use async_std::io::prelude::*;
use async_std::io::BufReader;
use async_std::net::TcpStream;
use tracing::{debug, info};

use crate::{Folder, FolderType, ImapError, ImapResult, MessageHeader, MessageFlags};
use crate::message::{EmailAddress, Envelope};

type TlsStream = async_native_tls::TlsStream<TcpStream>;

/// Simple IMAP client that works in any async context
pub struct SimpleImapClient {
    stream: Option<BufReader<TlsStream>>,
    tag_counter: u32,
}

impl SimpleImapClient {
    /// Create a new client
    pub fn new() -> Self {
        Self {
            stream: None,
            tag_counter: 0,
        }
    }

    fn next_tag(&mut self) -> String {
        self.tag_counter += 1;
        format!("A{:04}", self.tag_counter)
    }

    /// Connect to Gmail and authenticate with XOAUTH2
    pub async fn connect_gmail(&mut self, email: &str, access_token: &str) -> ImapResult<()> {
        self.connect_xoauth2("imap.gmail.com", 993, email, access_token).await
    }

    /// Connect to Microsoft/Outlook and authenticate with XOAUTH2
    pub async fn connect_outlook(&mut self, email: &str, access_token: &str) -> ImapResult<()> {
        self.connect_xoauth2("outlook.office365.com", 993, email, access_token).await
    }

    /// Connect to any IMAP server and authenticate with XOAUTH2
    pub async fn connect_xoauth2(
        &mut self,
        host: &str,
        port: u16,
        email: &str,
        access_token: &str,
    ) -> ImapResult<()> {
        info!("Connecting to {}:{}", host, port);

        // TCP connection
        let tcp_stream = TcpStream::connect(format!("{}:{}", host, port))
            .await
            .map_err(|e| ImapError::ConnectionFailed(e.to_string()))?;

        // TLS handshake
        let tls_connector = TlsConnector::new();
        let tls_stream = tls_connector
            .connect(host, tcp_stream)
            .await
            .map_err(|e| ImapError::TlsError(e.to_string()))?;

        debug!("TLS connection established");

        let mut stream = BufReader::new(tls_stream);

        // Read greeting
        let mut greeting = String::new();
        stream
            .read_line(&mut greeting)
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        debug!("Greeting: {}", greeting.trim());

        if !greeting.starts_with("* OK") {
            return Err(ImapError::ServerError(format!(
                "Unexpected greeting: {}",
                greeting
            )));
        }

        // Authenticate with XOAUTH2
        let auth_string = format!("user={}\x01auth=Bearer {}\x01\x01", email, access_token);
        let encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &auth_string);

        let tag = self.next_tag();
        let cmd = format!("{} AUTHENTICATE XOAUTH2 {}\r\n", tag, encoded);

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Read response until we get our tag
        let mut auth_ok = false;
        let mut error_msg = String::new();
        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("Auth response: {}", line.trim());

            // Check for continuation request (challenge) - send empty response
            if line.starts_with("+ ") {
                // Server is requesting more data, send empty line to get error details
                stream
                    .get_mut()
                    .write_all(b"\r\n")
                    .await
                    .map_err(|e| ImapError::ServerError(e.to_string()))?;
                continue;
            }

            if line.starts_with(&tag) {
                if line.contains("OK") {
                    auth_ok = true;
                } else {
                    error_msg = line.clone();
                }
                break;
            }
        }

        if !auth_ok {
            return Err(ImapError::AuthenticationFailed(
                format!("XOAUTH2 authentication failed: {}", error_msg.trim()),
            ));
        }

        info!("XOAUTH2 authentication successful");
        self.stream = Some(stream);
        Ok(())
    }

    /// Select a folder
    pub async fn select(&mut self, folder: &str) -> ImapResult<Folder> {
        let tag = self.next_tag();
        let cmd = format!("{} SELECT \"{}\"\r\n", tag, folder);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut exists = 0u32;
        let mut select_ok = false;

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("Select response: {}", line.trim());

            // Parse EXISTS count
            if line.contains(" EXISTS") {
                if let Some(n) = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                {
                    exists = n;
                }
            }

            if line.starts_with(&tag) {
                if line.contains("OK") {
                    select_ok = true;
                }
                break;
            }
        }

        if !select_ok {
            return Err(ImapError::FolderNotFound(folder.to_string()));
        }

        Ok(Folder {
            name: folder.to_string(),
            full_path: folder.to_string(),
            folder_type: FolderType::from_attributes_and_name(&[], folder),
            delimiter: Some('/'),
            attributes: vec![],
            uidvalidity: None,
            message_count: Some(exists),
            unread_count: None,
            uid_next: None,
        })
    }

    /// Fetch message headers
    pub async fn fetch_headers(&mut self, range: &str) -> ImapResult<Vec<MessageHeader>> {
        let tag = self.next_tag();
        // Fetch UID, FLAGS, and ENVELOPE
        let cmd = format!("{} FETCH {} (UID FLAGS ENVELOPE)\r\n", tag, range);

        // Collect raw response lines first
        let mut raw_lines = Vec::new();
        {
            let stream = self
                .stream
                .as_mut()
                .ok_or(ImapError::NotConnected)?;

            stream
                .get_mut()
                .write_all(cmd.as_bytes())
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            loop {
                let mut line = String::new();
                stream
                    .read_line(&mut line)
                    .await
                    .map_err(|e| ImapError::ServerError(e.to_string()))?;

                if line.starts_with(&tag) {
                    break;
                }

                // Collect FETCH responses
                if line.starts_with("* ") && line.contains("FETCH") {
                    raw_lines.push(line);
                }
            }
        }

        // Parse collected lines (stream borrow is released)
        let mut headers = Vec::new();
        for line in raw_lines {
            if let Some(header) = self.parse_fetch_response(&line) {
                headers.push(header);
            }
        }

        Ok(headers)
    }

    fn parse_fetch_response(&self, line: &str) -> Option<MessageHeader> {
        // Very simple parser - extract UID, FLAGS, and basic envelope info
        let uid = Self::extract_uid(line)?;
        let flag_strs = Self::extract_flags(line);
        let flag_refs: Vec<&str> = flag_strs.iter().map(|s| s.as_str()).collect();
        let flags = MessageFlags::from_imap_flags(&flag_refs);
        let envelope = Self::extract_envelope(line);

        Some(MessageHeader {
            uid,
            seq: 0, // Not available in simple parser
            envelope,
            flags,
            has_attachments: false,
            size: 0,
        })
    }

    fn extract_uid(line: &str) -> Option<u32> {
        // Look for "UID 12345"
        if let Some(idx) = line.find("UID ") {
            let rest = &line[idx + 4..];
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            return rest[..end].parse().ok();
        }
        None
    }

    fn extract_flags(line: &str) -> Vec<String> {
        // Look for "FLAGS (\Seen \Flagged)"
        let mut flags = Vec::new();
        if let Some(start) = line.find("FLAGS (") {
            let rest = &line[start + 7..];
            if let Some(end) = rest.find(')') {
                let flags_str = &rest[..end];
                for flag in flags_str.split_whitespace() {
                    flags.push(flag.to_string());
                }
            }
        }
        flags
    }

    fn extract_envelope(line: &str) -> Envelope {
        // ENVELOPE format: (date subject from sender reply-to to cc bcc in-reply-to message-id)
        // Each address field is: ((name route mailbox host) ...) or NIL
        let mut envelope = Envelope::default();

        if let Some(start) = line.find("ENVELOPE (") {
            let rest = &line[start + 10..];

            // Parse envelope parts properly handling nested parens and quotes
            let parts = Self::parse_envelope_parts(rest);

            // parts[0] = date, parts[1] = subject, parts[2] = from addresses
            if let Some(date) = parts.get(0) {
                if *date != "NIL" {
                    envelope.date = Some(date.clone());
                }
            }
            if let Some(subject) = parts.get(1) {
                if *subject != "NIL" {
                    envelope.subject = Some(subject.clone());
                }
            }
            // From is the 3rd element (index 2)
            if let Some(from_str) = parts.get(2) {
                if let Some(addrs) = Self::parse_address_list_from_envelope(from_str) {
                    envelope.from = addrs;
                }
            }
        }

        envelope
    }

    /// Parse envelope parts, handling quoted strings, NIL, and nested parentheses
    fn parse_envelope_parts(s: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            // Skip whitespace
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            match chars[i] {
                '"' => {
                    // Quoted string
                    i += 1;
                    let start = i;
                    while i < chars.len() && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < chars.len() {
                            i += 2; // Skip escaped char
                        } else {
                            i += 1;
                        }
                    }
                    let value: String = chars[start..i].iter().collect();
                    parts.push(value);
                    i += 1; // Skip closing quote
                }
                '(' => {
                    // Nested structure (like address list)
                    let start = i;
                    let mut depth = 1;
                    i += 1;
                    while i < chars.len() && depth > 0 {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            '"' => {
                                // Skip quoted string inside
                                i += 1;
                                while i < chars.len() && chars[i] != '"' {
                                    if chars[i] == '\\' && i + 1 < chars.len() {
                                        i += 1;
                                    }
                                    i += 1;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let value: String = chars[start..i].iter().collect();
                    parts.push(value);
                }
                'N' if i + 2 < chars.len() => {
                    // Check for NIL
                    let word: String = chars[i..].iter().take(3).collect();
                    if word == "NIL" {
                        parts.push("NIL".to_string());
                        i += 3;
                    } else {
                        i += 1;
                    }
                }
                ')' => {
                    // End of envelope
                    break;
                }
                _ => {
                    i += 1;
                }
            }
        }

        parts
    }

    /// Parse address list from envelope format: ((name route mailbox host) ...)
    fn parse_address_list_from_envelope(s: &str) -> Option<Vec<EmailAddress>> {
        if s == "NIL" || s.is_empty() {
            return None;
        }

        let mut addresses = Vec::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        // Skip outer opening paren
        while i < chars.len() && chars[i] != '(' {
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        i += 1; // Skip first (

        // Now parse each address: (name route mailbox host)
        while i < chars.len() {
            // Skip whitespace
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }

            if i >= chars.len() || chars[i] == ')' {
                break;
            }

            if chars[i] == '(' {
                // Parse one address
                i += 1;
                let mut addr_parts: Vec<String> = Vec::new();

                // Parse 4 parts: name, route, mailbox, host
                for _ in 0..4 {
                    // Skip whitespace
                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }

                    if i >= chars.len() {
                        break;
                    }

                    if chars[i] == '"' {
                        // Quoted string
                        i += 1;
                        let start = i;
                        while i < chars.len() && chars[i] != '"' {
                            if chars[i] == '\\' && i + 1 < chars.len() {
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        let value: String = chars[start..i].iter().collect();
                        addr_parts.push(value);
                        i += 1;
                    } else if i + 2 < chars.len() {
                        let word: String = chars[i..].iter().take(3).collect();
                        if word == "NIL" {
                            addr_parts.push("".to_string());
                            i += 3;
                        } else {
                            // Unknown, skip
                            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != ')' {
                                i += 1;
                            }
                        }
                    } else {
                        break;
                    }
                }

                // Skip to closing paren of this address
                while i < chars.len() && chars[i] != ')' {
                    i += 1;
                }
                i += 1; // Skip )

                // addr_parts: [name, route, mailbox, host]
                if addr_parts.len() >= 4 {
                    let name = if addr_parts[0].is_empty() { None } else { Some(addr_parts[0].clone()) };
                    let mailbox = &addr_parts[2];
                    let host = &addr_parts[3];

                    if !mailbox.is_empty() && !host.is_empty() {
                        addresses.push(EmailAddress {
                            name,
                            address: format!("{}@{}", mailbox, host),
                        });
                    } else if !mailbox.is_empty() {
                        // Some servers put full email in mailbox
                        addresses.push(EmailAddress {
                            name,
                            address: mailbox.clone(),
                        });
                    }
                }
            } else {
                i += 1;
            }
        }

        if addresses.is_empty() {
            None
        } else {
            Some(addresses)
        }
    }

    /// Fetch message body by UID
    pub async fn fetch_body(&mut self, uid: u32) -> ImapResult<String> {
        let tag = self.next_tag();
        // Use BODY.PEEK[] to avoid marking the message as read
        let cmd = format!("{} UID FETCH {} BODY.PEEK[]\r\n", tag, uid);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut body_bytes: Vec<u8> = Vec::new();

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            // Check for our tag (completion)
            if line.starts_with(&tag) {
                break;
            }

            // Check for literal start: * N FETCH (BODY[] {SIZE}
            if let Some(literal_start) = line.find('{') {
                if let Some(literal_end) = line.find('}') {
                    if let Ok(size) = line[literal_start + 1..literal_end].parse::<usize>() {
                        debug!("Reading literal of {} bytes", size);

                        // Read exactly 'size' bytes of literal data
                        let mut literal_buf = vec![0u8; size];
                        let inner_stream = stream.get_mut();

                        use async_std::io::ReadExt;
                        inner_stream
                            .read_exact(&mut literal_buf)
                            .await
                            .map_err(|e| ImapError::ServerError(format!("Failed to read literal: {}", e)))?;

                        body_bytes = literal_buf;

                        // Read the closing line (contains ")" and possibly more)
                        let mut closing_line = String::new();
                        stream
                            .read_line(&mut closing_line)
                            .await
                            .map_err(|e| ImapError::ServerError(e.to_string()))?;

                        debug!("Literal closing line: {}", closing_line.trim());
                    }
                }
            }
        }

        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        debug!("Fetched body: {} bytes", body.len());
        Ok(body)
    }

    /// List folders
    pub async fn list_folders(&mut self) -> ImapResult<Vec<Folder>> {
        let tag = self.next_tag();
        let cmd = format!("{} LIST \"\" \"*\"\r\n", tag);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut folders = Vec::new();

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("LIST response: {}", line.trim());

            if line.starts_with(&tag) {
                break;
            }

            // Parse LIST response: * LIST (\HasNoChildren) "/" "INBOX"
            if line.starts_with("* LIST ") {
                if let Some(folder) = Self::parse_list_response(&line) {
                    folders.push(folder);
                }
            }
        }

        Ok(folders)
    }

    fn parse_list_response(line: &str) -> Option<Folder> {
        // Format: * LIST (\attr1 \attr2) "/" "folder name"
        let rest = line.strip_prefix("* LIST ")?;

        // Extract attributes
        let attr_start = rest.find('(')?;
        let attr_end = rest.find(')')?;
        let attrs_str = &rest[attr_start + 1..attr_end];
        let attributes: Vec<String> = attrs_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        // Find delimiter and folder name
        let after_attrs = &rest[attr_end + 1..].trim();

        // Parse delimiter (could be "/" or NIL)
        let delimiter = if after_attrs.starts_with("NIL") {
            None
        } else if after_attrs.starts_with('"') {
            after_attrs.chars().nth(1)
        } else {
            None
        };

        // Find folder name (last quoted string)
        let name_start = after_attrs.rfind('"')?;
        let name_end = after_attrs[..name_start].rfind('"')?;
        let name = &after_attrs[name_end + 1..name_start];

        // Check for \Noselect attribute
        let is_noselect = attributes.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect"));

        if is_noselect {
            return None; // Skip non-selectable folders
        }

        Some(Folder {
            name: name.split(delimiter.unwrap_or('/')).last().unwrap_or(name).to_string(),
            full_path: name.to_string(),
            folder_type: FolderType::from_attributes_and_name(&attributes, name),
            delimiter,
            attributes,
            uidvalidity: None,
            message_count: None,
            unread_count: None,
            uid_next: None,
        })
    }

    /// Check if connection is alive with NOOP
    pub async fn noop(&mut self) -> ImapResult<()> {
        if self.stream.is_none() {
            return Err(ImapError::NotConnected);
        }

        let tag = self.next_tag();
        let cmd = format!("{} NOOP\r\n", tag);

        let stream = self.stream.as_mut().unwrap();
        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Read response
        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            if line.starts_with(&tag) {
                if line.contains("OK") {
                    return Ok(());
                } else {
                    return Err(ImapError::ServerError(format!(
                        "NOOP failed: {}",
                        line.trim()
                    )));
                }
            }
        }
    }

    /// Check if the client has a connection
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    /// Logout
    pub async fn logout(&mut self) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} LOGOUT\r\n", tag);
        if let Some(stream) = self.stream.as_mut() {
            let _ = stream.get_mut().write_all(cmd.as_bytes()).await;
        }
        self.stream = None;
        Ok(())
    }
}

impl Default for SimpleImapClient {
    fn default() -> Self {
        Self::new()
    }
}
