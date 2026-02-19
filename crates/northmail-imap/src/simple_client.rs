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

use std::time::Duration;

type TlsStream = async_native_tls::TlsStream<TcpStream>;

/// Event returned from IDLE mode
#[derive(Debug, Clone, PartialEq)]
pub enum IdleEvent {
    /// New messages exist (reported count from EXISTS response)
    NewMessages(u32),
    /// A message was expunged (sequence number)
    Expunge(u32),
    /// Flags changed on a message
    FlagsChanged,
    /// IDLE timed out (for keepalive)
    Timeout,
    /// Server closed connection
    ServerBye,
}

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

    /// Connect to any IMAP server and authenticate with LOGIN (password)
    pub async fn connect_login(
        &mut self,
        host: &str,
        port: u16,
        username: &str,
        password: &str,
    ) -> ImapResult<()> {
        info!("Connecting to {}:{}", host, port);

        let tcp_stream = TcpStream::connect(format!("{}:{}", host, port))
            .await
            .map_err(|e| ImapError::ConnectionFailed(e.to_string()))?;

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

        // LOGIN command - quote username and password
        let tag = self.next_tag();
        let cmd = format!("{} LOGIN \"{}\" \"{}\"\r\n", tag,
            username.replace('\\', "\\\\").replace('"', "\\\""),
            password.replace('\\', "\\\\").replace('"', "\\\""));

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Read response
        let mut auth_ok = false;
        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("Login response: {}", line.trim());

            if line.starts_with(&tag) {
                if line.contains("OK") {
                    auth_ok = true;
                }
                break;
            }
        }

        if !auth_ok {
            return Err(ImapError::AuthenticationFailed(
                "LOGIN authentication failed".to_string(),
            ));
        }

        info!("LOGIN authentication successful");
        self.stream = Some(stream);
        Ok(())
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
        // Fetch UID, FLAGS, ENVELOPE, and BODYSTRUCTURE (for attachment detection)
        let cmd = format!("{} FETCH {} (UID FLAGS ENVELOPE BODYSTRUCTURE)\r\n", tag, range);

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
        let has_attachments = Self::detect_attachments(line);

        Some(MessageHeader {
            uid,
            seq: 0, // Not available in simple parser
            envelope,
            flags,
            has_attachments,
            size: 0,
        })
    }

    /// Detect attachments from BODYSTRUCTURE in the raw FETCH response.
    /// Checks the BODYSTRUCTURE portion for:
    /// 1. Explicit "attachment" disposition
    /// 2. Non-text/non-multipart MIME primary types (application, image, audio, video)
    ///    which indicate file attachments even without explicit disposition
    fn detect_attachments(line: &str) -> bool {
        // Only search the BODYSTRUCTURE portion to avoid false positives from envelope fields
        let search_area = if let Some(idx) = line.find("BODYSTRUCTURE ") {
            &line[idx..]
        } else {
            return false;
        };
        let lower = search_area.to_ascii_lowercase();

        // Explicit attachment disposition
        if lower.contains("\"attachment\"") {
            // But exclude S/MIME signatures even if marked as attachment
            if lower.contains("\"pkcs7-signature\"") || lower.contains("\"pgp-signature\"")
                || lower.contains("\"x-pkcs7-signature\"") || lower.contains("\"pkcs7-mime\"")
            {
                // Only count as attachment if there are OTHER attachment-like types too
                return lower.contains("\"image\"") || lower.contains("\"audio\"")
                    || lower.contains("\"video\"")
                    || (lower.contains("\"application\"")
                        && !lower.contains("\"pkcs7-signature\"")
                        && !lower.contains("\"x-pkcs7-signature\"")
                        && !lower.contains("\"pgp-signature\"")
                        && !lower.contains("\"pkcs7-mime\""));
            }
            return true;
        }

        // Non-text MIME types, but exclude S/MIME signatures and inline images
        // S/MIME: pkcs7-signature, x-pkcs7-signature, pgp-signature, pkcs7-mime
        // Inline images: have Content-ID (appear as "image" type without "attachment" disposition)
        if lower.contains("\"application\"") {
            // Only count application/* as attachment if it's NOT a signature type
            if !lower.contains("\"pkcs7-signature\"") && !lower.contains("\"x-pkcs7-signature\"")
                && !lower.contains("\"pgp-signature\"") && !lower.contains("\"pkcs7-mime\"")
            {
                return true;
            }
        }

        // Images/audio/video: only count if explicitly marked as "attachment" disposition
        // (inline images with Content-ID should not count as attachments)
        // Since we already checked for "attachment" above, these would be inline
        // Only flag if there's no Content-ID-like structure nearby
        if lower.contains("\"audio\"") || lower.contains("\"video\"") {
            return true;
        }

        // For images, only count as attachment if there's an explicit "attachment" disposition
        // (already handled above). Images without "attachment" are likely inline.
        false
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
            // To is the 6th element (index 5) - ENVELOPE format: (date subject from sender reply-to to ...)
            if let Some(to_str) = parts.get(5) {
                if let Some(addrs) = Self::parse_address_list_from_envelope(to_str) {
                    envelope.to = addrs;
                }
            }
            // CC is the 7th element (index 6)
            if let Some(cc_str) = parts.get(6) {
                if let Some(addrs) = Self::parse_address_list_from_envelope(cc_str) {
                    envelope.cc = addrs;
                }
            }
            // Message-ID is the 10th element (index 9)
            // ENVELOPE format: (date subject from sender reply-to to cc bcc in-reply-to message-id)
            if let Some(msg_id) = parts.get(9) {
                if *msg_id != "NIL" {
                    envelope.message_id = Some(msg_id.clone());
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
                    // Quoted string — unescape \" and \\
                    i += 1;
                    let mut value = String::new();
                    while i < chars.len() && chars[i] != '"' {
                        if chars[i] == '\\' && i + 1 < chars.len() {
                            i += 1; // skip backslash, take next char literally
                        }
                        value.push(chars[i]);
                        i += 1;
                    }
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
                        // Quoted string — collect and unescape \" and \\
                        i += 1;
                        let mut value = String::new();
                        while i < chars.len() && chars[i] != '"' {
                            if chars[i] == '\\' && i + 1 < chars.len() {
                                i += 1; // skip backslash, take next char literally
                            }
                            value.push(chars[i]);
                            i += 1;
                        }
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

    /// Fetch message headers by UID range (uses UID FETCH instead of FETCH)
    pub async fn uid_fetch_headers(&mut self, range: &str) -> ImapResult<Vec<MessageHeader>> {
        let tag = self.next_tag();
        let cmd = format!("{} UID FETCH {} (UID FLAGS ENVELOPE BODYSTRUCTURE)\r\n", tag, range);

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

                if line.starts_with("* ") && line.contains("FETCH") {
                    raw_lines.push(line);
                }
            }
        }

        let mut headers = Vec::new();
        for line in raw_lines {
            if let Some(header) = self.parse_fetch_response(&line) {
                headers.push(header);
            }
        }

        Ok(headers)
    }

    /// Fetch flags for all messages by UID range
    /// Returns Vec<(uid, is_read, is_starred)>
    pub async fn uid_fetch_flags(&mut self, range: &str) -> ImapResult<Vec<(u32, bool, bool)>> {
        let tag = self.next_tag();
        let cmd = format!("{} UID FETCH {} (UID FLAGS)\r\n", tag, range);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut results = Vec::new();

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            if line.starts_with(&tag) {
                break;
            }

            if line.starts_with("* ") && line.contains("FETCH") {
                if let Some(uid) = Self::extract_uid(&line) {
                    let flag_strs = Self::extract_flags(&line);
                    let is_read = flag_strs.iter().any(|f| f.eq_ignore_ascii_case("\\Seen"));
                    let is_starred = flag_strs.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged"));
                    results.push((uid, is_read, is_starred));
                }
            }
        }

        Ok(results)
    }

    /// Fetch message body by UID
    pub async fn fetch_body(&mut self, uid: u32) -> ImapResult<String> {
        use std::time::Duration;
        use async_std::future::timeout;

        let tag = self.next_tag();
        // Use BODY.PEEK[] to avoid marking the message as read
        let cmd = format!("{} UID FETCH {} BODY.PEEK[]\r\n", tag, uid);

        debug!("fetch_body: sending command: {} UID FETCH {} BODY.PEEK[]", tag, uid);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        debug!("fetch_body: command sent, waiting for response");

        let mut body_bytes: Vec<u8> = Vec::new();
        let read_timeout = Duration::from_secs(30);

        loop {
            let mut line = String::new();
            debug!("fetch_body: waiting for line (timeout: 30s)...");

            let read_result = timeout(read_timeout, stream.read_line(&mut line)).await;
            match read_result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    return Err(ImapError::ServerError(format!("Read error: {}", e)));
                }
                Err(_) => {
                    return Err(ImapError::ServerError(format!(
                        "Timeout waiting for response to UID FETCH {} - message may not exist",
                        uid
                    )));
                }
            }

            debug!("fetch_body: received line: {} chars, starts with: '{}'",
                   line.len(),
                   line.chars().take(60).collect::<String>().replace('\r', "\\r").replace('\n', "\\n"));

            // Check for our tag (completion)
            if line.starts_with(&tag) {
                debug!("fetch_body: got completion tag");
                break;
            }

            // Check for literal start: * N FETCH (BODY[] {SIZE}
            if let Some(literal_start) = line.find('{') {
                if let Some(literal_end) = line.find('}') {
                    if let Ok(size) = line[literal_start + 1..literal_end].parse::<usize>() {
                        debug!("fetch_body: reading literal of {} bytes", size);

                        // Read exactly 'size' bytes of literal data
                        // IMPORTANT: Read from BufReader (stream), NOT stream.get_mut(),
                        // because BufReader may have already buffered part of the literal
                        let mut literal_buf = vec![0u8; size];

                        use async_std::io::ReadExt;
                        let read_exact_result = timeout(
                            Duration::from_secs(60),  // Longer timeout for large bodies
                            stream.read_exact(&mut literal_buf)
                        ).await;

                        match read_exact_result {
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => {
                                return Err(ImapError::ServerError(format!("Failed to read literal: {}", e)));
                            }
                            Err(_) => {
                                return Err(ImapError::ServerError("Timeout reading message body".to_string()));
                            }
                        }

                        debug!("fetch_body: read {} bytes of literal data", literal_buf.len());
                        body_bytes = literal_buf;

                        // Read the closing line (contains ")" and possibly more)
                        let mut closing_line = String::new();
                        let close_result = timeout(read_timeout, stream.read_line(&mut closing_line)).await;
                        match close_result {
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => {
                                return Err(ImapError::ServerError(format!("Read error: {}", e)));
                            }
                            Err(_) => {
                                return Err(ImapError::ServerError("Timeout reading closing line".to_string()));
                            }
                        }

                        debug!("fetch_body: closing line: {}", closing_line.trim());
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
        // Format: * LIST (\attr1 \attr2) "delimiter" "folder name"
        //     or: * LIST (\attr1 \attr2) NIL "folder name"
        let rest = line.strip_prefix("* LIST ")?;

        // Extract attributes between ( and )
        let attr_start = rest.find('(')?;
        let attr_end = rest.find(')')?;
        let attrs_str = &rest[attr_start + 1..attr_end];
        let attributes: Vec<String> = attrs_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        // Everything after the closing paren: e.g. ` "/" "INBOX"` or ` NIL "INBOX"`
        let after_attrs = rest[attr_end + 1..].trim();

        // Parse delimiter and find where folder name starts
        let (delimiter, folder_part) = if after_attrs.starts_with("NIL") {
            (None, after_attrs[3..].trim())
        } else if after_attrs.starts_with('"') {
            // Delimiter is a quoted single character like "/"
            let delim_char = after_attrs.chars().nth(1);
            // Skip past the delimiter string (e.g. `"/"`) to get to the folder name
            if let Some(close) = after_attrs[1..].find('"') {
                (delim_char, after_attrs[close + 2..].trim())
            } else {
                (None, after_attrs)
            }
        } else {
            (None, after_attrs)
        };

        // Extract folder name from the remaining part — should be a quoted string
        let folder_name = if folder_part.starts_with('"') {
            // Find the closing quote (handle escaped quotes)
            let inner = &folder_part[1..];
            let mut end = 0;
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    // Skip escaped character
                    chars.next();
                    end += 2;
                } else if c == '"' {
                    break;
                } else {
                    end += c.len_utf8();
                }
            }
            &inner[..end]
        } else {
            // Unquoted folder name (some servers do this)
            folder_part.trim()
        };

        // Check for \Noselect attribute
        let is_noselect = attributes.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect"));
        if is_noselect {
            return None;
        }

        // Skip empty or root-delimiter-only folder names
        if folder_name.is_empty()
            || (folder_name.len() == 1 && delimiter == Some(folder_name.chars().next().unwrap_or('/')))
        {
            debug!("Skipping root/empty folder: {:?}", folder_name);
            return None;
        }

        Some(Folder {
            name: folder_name
                .split(delimiter.unwrap_or('/'))
                .last()
                .unwrap_or(folder_name)
                .to_string(),
            full_path: folder_name.to_string(),
            folder_type: FolderType::from_attributes_and_name(&attributes, folder_name),
            delimiter,
            attributes,
            uidvalidity: None,
            message_count: None,
            unread_count: None,
            uid_next: None,
        })
    }

    /// Get STATUS for a folder (MESSAGES and UNSEEN counts)
    /// Returns (message_count, unseen_count)
    pub async fn folder_status(&mut self, folder: &str) -> ImapResult<(u32, u32)> {
        let tag = self.next_tag();
        let cmd = format!("{} STATUS \"{}\" (MESSAGES UNSEEN)\r\n", tag, folder);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        let mut messages = 0u32;
        let mut unseen = 0u32;

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("STATUS response: {}", line.trim());

            // Parse: * STATUS "folder" (MESSAGES 42 UNSEEN 5)
            if line.starts_with("* STATUS ") {
                if let Some(paren_start) = line.rfind('(') {
                    let status_data = &line[paren_start + 1..];
                    let parts: Vec<&str> = status_data.split_whitespace().collect();
                    for i in (0..parts.len()).step_by(2) {
                        if i + 1 < parts.len() {
                            let val = parts[i + 1].trim_end_matches(')').parse().unwrap_or(0);
                            match parts[i] {
                                "MESSAGES" => messages = val,
                                "UNSEEN" => unseen = val,
                                _ => {}
                            }
                        }
                    }
                }
            }

            if line.starts_with(&tag) {
                break;
            }
        }

        Ok((messages, unseen))
    }

    /// Pipelined batch STATUS for multiple folders.
    /// Sends ALL STATUS commands before reading any responses.
    /// For N folders: N sequential round trips → 1 pipelined batch.
    /// Returns Vec<(folder_path, message_count, unseen_count)>.
    pub async fn batch_folder_status(&mut self, folders: &[&str]) -> ImapResult<Vec<(String, u32, u32)>> {
        if folders.is_empty() {
            return Ok(Vec::new());
        }

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        // Phase 1: Send all STATUS commands without waiting for responses
        let mut tags: Vec<(String, String)> = Vec::with_capacity(folders.len()); // (tag, folder_path)
        for folder in folders {
            self.tag_counter += 1;
            let tag = format!("A{:04}", self.tag_counter);
            let cmd = format!("{} STATUS \"{}\" (MESSAGES UNSEEN)\r\n", tag, folder);
            stream
                .get_mut()
                .write_all(cmd.as_bytes())
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;
            tags.push((tag, folder.to_string()));
        }

        // Flush all commands at once
        stream
            .get_mut()
            .flush()
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Phase 2: Read all responses in tag order.
        // IMAP processes pipelined commands sequentially (RFC 3501 §3),
        // so the * STATUS response before each tagged OK belongs to that command.
        // This avoids case-sensitivity issues (e.g. Outlook returns "Inbox" not "INBOX").
        let mut results: Vec<(String, u32, u32)> = Vec::with_capacity(folders.len());
        let mut completed_tags = 0;
        // Accumulate the most recent STATUS data seen before each tagged response
        let mut pending_messages = 0u32;
        let mut pending_unseen = 0u32;
        let mut got_status = false;

        while completed_tags < tags.len() {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("Batch STATUS response: {}", line.trim());

            // Parse: * STATUS "folder" (MESSAGES 42 UNSEEN 5)
            if line.starts_with("* STATUS ") {
                if let Some((_, messages, unseen)) = Self::parse_status_line(&line) {
                    pending_messages = messages;
                    pending_unseen = unseen;
                    got_status = true;
                }
            }

            // Check if this is a tagged response (OK or BAD)
            let (tag, folder_path) = &tags[completed_tags];
            if line.starts_with(tag.as_str()) {
                if got_status {
                    results.push((folder_path.clone(), pending_messages, pending_unseen));
                } else {
                    // BAD or no STATUS line — use (0, 0)
                    results.push((folder_path.clone(), 0, 0));
                }
                completed_tags += 1;
                got_status = false;
                pending_messages = 0;
                pending_unseen = 0;
            }
        }

        Ok(results)
    }

    /// Parse a STATUS response line: * STATUS "folder" (MESSAGES 42 UNSEEN 5)
    fn parse_status_line(line: &str) -> Option<(String, u32, u32)> {
        // Extract folder name (quoted)
        let after_status = line.strip_prefix("* STATUS ")?;

        // Folder name can be quoted or unquoted
        let (folder_name, rest) = if after_status.starts_with('"') {
            let inner = &after_status[1..];
            let end = inner.find('"')?;
            (&inner[..end], &inner[end + 1..])
        } else {
            let end = after_status.find(' ')?;
            (&after_status[..end], &after_status[end..])
        };

        // Parse counts from (MESSAGES N UNSEEN M)
        let mut messages = 0u32;
        let mut unseen = 0u32;
        if let Some(paren_start) = rest.rfind('(') {
            let data = &rest[paren_start + 1..];
            let parts: Vec<&str> = data.split_whitespace().collect();
            for i in (0..parts.len()).step_by(2) {
                if i + 1 < parts.len() {
                    let val = parts[i + 1].trim_end_matches(')').parse().unwrap_or(0);
                    match parts[i] {
                        "MESSAGES" => messages = val,
                        "UNSEEN" => unseen = val,
                        _ => {}
                    }
                }
            }
        }

        Some((folder_name.to_string(), messages, unseen))
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

    /// APPEND a message to a folder, returning the UID if the server provides APPENDUID
    pub async fn append(
        &mut self,
        folder: &str,
        flags: &[&str],
        message_data: &[u8],
    ) -> ImapResult<Option<u32>> {
        let tag = self.next_tag();
        let flags_str = if flags.is_empty() {
            String::new()
        } else {
            format!(" ({})", flags.join(" "))
        };
        let cmd = format!(
            "{} APPEND \"{}\"{} {{{}}}\r\n",
            tag,
            folder,
            flags_str,
            message_data.len()
        );

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        // Send the APPEND command with literal size
        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Wait for continuation response "+ "
        let mut line = String::new();
        stream
            .read_line(&mut line)
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        debug!("APPEND continuation: {}", line.trim());

        if !line.starts_with('+') {
            return Err(ImapError::ServerError(format!(
                "Expected continuation, got: {}",
                line.trim()
            )));
        }

        // Send the literal message data followed by CRLF
        stream
            .get_mut()
            .write_all(message_data)
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;
        stream
            .get_mut()
            .write_all(b"\r\n")
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Read the tagged response
        let mut uid = None;
        loop {
            let mut resp = String::new();
            stream
                .read_line(&mut resp)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("APPEND response: {}", resp.trim());

            if resp.starts_with(&tag) {
                if !resp.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "APPEND failed: {}",
                        resp.trim()
                    )));
                }
                // Parse [APPENDUID uidvalidity uid] from OK response
                if let Some(start) = resp.find("[APPENDUID ") {
                    let rest = &resp[start + 11..];
                    if let Some(end) = rest.find(']') {
                        let parts: Vec<&str> = rest[..end].split_whitespace().collect();
                        if parts.len() == 2 {
                            uid = parts[1].parse().ok();
                        }
                    }
                }
                break;
            }
        }

        debug!("APPEND successful, uid: {:?}", uid);
        Ok(uid)
    }

    /// Add or remove flags on a message by UID
    /// `add` = true for +FLAGS, false for -FLAGS
    pub async fn uid_store_flags(&mut self, uid: u32, flags: &str, add: bool) -> ImapResult<()> {
        let tag = self.next_tag();
        let op = if add { "+" } else { "-" };
        let cmd = format!("{} UID STORE {} {}FLAGS ({})\r\n", tag, uid, op, flags);

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

            debug!("UID STORE response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "UID STORE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Copy a message to another folder by UID
    pub async fn uid_copy(&mut self, uid: u32, dest_folder: &str) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} UID COPY {} \"{}\"\r\n", tag, uid, dest_folder);

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

            debug!("UID COPY response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "UID COPY failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Expunge deleted messages from the current folder
    pub async fn expunge(&mut self) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} EXPUNGE\r\n", tag);

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

            debug!("EXPUNGE response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "EXPUNGE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// UID EXPUNGE - expunge only the specified UID (requires UIDPLUS extension)
    /// This is more reliable than EXPUNGE for Gmail and other servers
    pub async fn uid_expunge(&mut self, uid: u32) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} UID EXPUNGE {}\r\n", tag, uid);

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

            debug!("UID EXPUNGE response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "UID EXPUNGE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Set \Deleted flag on a message by UID and EXPUNGE it
    pub async fn uid_store_deleted_and_expunge(&mut self, uid: u32) -> ImapResult<()> {
        // Generate both tags before borrowing stream
        let tag1 = self.next_tag();
        let tag2 = self.next_tag();
        let cmd1 = format!("{} UID STORE {} +FLAGS (\\Deleted)\r\n", tag1, uid);
        let cmd2 = format!("{} EXPUNGE\r\n", tag2);

        let stream = self
            .stream
            .as_mut()
            .ok_or(ImapError::NotConnected)?;

        // UID STORE +FLAGS (\Deleted)
        stream
            .get_mut()
            .write_all(cmd1.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("STORE response: {}", line.trim());

            if line.starts_with(&tag1) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "UID STORE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        // EXPUNGE
        stream
            .get_mut()
            .write_all(cmd2.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        loop {
            let mut line = String::new();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            debug!("EXPUNGE response: {}", line.trim());

            if line.starts_with(&tag2) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "EXPUNGE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Create a new folder (mailbox) on the server
    /// Empty a folder by marking all messages as \Deleted and expunging
    pub async fn empty_folder(&mut self, folder_path: &str) -> ImapResult<()> {
        // Select the folder
        self.select(folder_path).await?;

        // Mark all messages as \Deleted (1:* means all messages)
        let tag = self.next_tag();
        let cmd = format!("{} STORE 1:* +FLAGS (\\Deleted)\r\n", tag);

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

            debug!("STORE response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    // If folder is empty, STORE 1:* may fail — that's OK
                    if line.contains("no matching messages") || line.contains("No matching") {
                        break;
                    }
                    return Err(ImapError::ServerError(format!(
                        "STORE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        // Expunge to permanently remove
        self.expunge().await?;

        Ok(())
    }

    pub async fn create_folder(&mut self, folder_path: &str) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} CREATE \"{}\"\r\n", tag, folder_path);

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

            debug!("CREATE response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "CREATE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Rename a folder (mailbox) on the server
    pub async fn rename_folder(&mut self, from: &str, to: &str) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} RENAME \"{}\" \"{}\"\r\n", tag, from, to);

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

            debug!("RENAME response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "RENAME failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Delete a folder (mailbox) from the server
    pub async fn delete_folder(&mut self, folder_path: &str) -> ImapResult<()> {
        let tag = self.next_tag();
        let cmd = format!("{} DELETE \"{}\"\r\n", tag, folder_path);

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

            debug!("DELETE response: {}", line.trim());

            if line.starts_with(&tag) {
                if !line.contains("OK") {
                    return Err(ImapError::ServerError(format!(
                        "DELETE failed: {}",
                        line.trim()
                    )));
                }
                break;
            }
        }

        Ok(())
    }

    /// Logout
    /// Enter IDLE mode and wait for server events or timeout
    ///
    /// IDLE allows the server to push notifications about mailbox changes.
    /// The client must call `idle_done()` to exit IDLE mode before sending
    /// other commands.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait before returning `IdleEvent::Timeout`
    ///
    /// # Returns
    /// The first event received from the server, or `Timeout` if no event
    /// arrives within the specified duration.
    pub async fn idle(&mut self, timeout: Duration) -> ImapResult<IdleEvent> {
        // Get tag before borrowing stream to avoid borrow checker issues
        let tag = self.next_tag();
        let cmd = format!("{} IDLE\r\n", tag);

        let stream = self.stream.as_mut().ok_or_else(|| {
            ImapError::ServerError("Not connected".to_string())
        })?;

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Wait for continuation response (+)
        let mut line = String::new();
        stream
            .read_line(&mut line)
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        debug!("IDLE response: {}", line.trim());

        if !line.starts_with('+') {
            return Err(ImapError::ServerError(format!(
                "Expected '+' continuation, got: {}",
                line.trim()
            )));
        }

        // Clear line before entering event loop (read_line appends!)
        line.clear();

        // Now we're in IDLE mode - wait for untagged responses
        // Use a timeout to allow periodic keepalive
        let start = std::time::Instant::now();

        loop {
            // Check if we've exceeded timeout
            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Ok(IdleEvent::Timeout);
            }

            // Calculate remaining timeout
            let remaining = timeout - elapsed;

            // Try to read a line with timeout
            let read_future = stream.read_line(&mut line);
            let result = async_std::future::timeout(remaining, read_future).await;

            match result {
                Ok(Ok(0)) => {
                    // Connection closed
                    return Ok(IdleEvent::ServerBye);
                }
                Ok(Ok(_)) => {
                    let trimmed = line.trim();
                    info!("IDLE received: {}", trimmed);

                    // Parse untagged response
                    if trimmed.starts_with("* BYE") {
                        return Ok(IdleEvent::ServerBye);
                    } else if let Some(rest) = trimmed.strip_prefix("* ") {
                        // Parse "* N EXISTS", "* N EXPUNGE", "* N FETCH (FLAGS ...)"
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        if parts.len() >= 2 {
                            if let Ok(num) = parts[0].parse::<u32>() {
                                match parts[1].to_uppercase().as_str() {
                                    "EXISTS" => {
                                        info!("IDLE: Detected EXISTS event with count {}", num);
                                        return Ok(IdleEvent::NewMessages(num));
                                    }
                                    "RECENT" => {
                                        // Some servers send RECENT for new mail
                                        if num > 0 {
                                            info!("IDLE: Detected RECENT event with count {}", num);
                                            return Ok(IdleEvent::NewMessages(num));
                                        }
                                    }
                                    "EXPUNGE" => return Ok(IdleEvent::Expunge(num)),
                                    "FETCH" => {
                                        // Flags updated
                                        return Ok(IdleEvent::FlagsChanged);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    // Clear line for next read
                    line.clear();
                }
                Ok(Err(e)) => {
                    return Err(ImapError::ServerError(format!("Read error: {}", e)));
                }
                Err(_) => {
                    // Timeout
                    return Ok(IdleEvent::Timeout);
                }
            }
        }
    }

    /// Exit IDLE mode by sending DONE
    ///
    /// This must be called before sending any other IMAP commands after `idle()`.
    pub async fn idle_done(&mut self) -> ImapResult<()> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            ImapError::ServerError("Not connected".to_string())
        })?;

        // Send DONE to exit IDLE
        stream
            .get_mut()
            .write_all(b"DONE\r\n")
            .await
            .map_err(|e| ImapError::ServerError(e.to_string()))?;

        // Read the tagged response
        let mut line = String::new();
        loop {
            line.clear();
            stream
                .read_line(&mut line)
                .await
                .map_err(|e| ImapError::ServerError(e.to_string()))?;

            let trimmed = line.trim();
            debug!("IDLE DONE response: {}", trimmed);

            // Look for tagged response (OK or error)
            if trimmed.contains(" OK") || trimmed.contains(" NO") || trimmed.contains(" BAD") {
                break;
            }
        }

        Ok(())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_list_gmail_inbox() {
        let line = r#"* LIST (\HasNoChildren) "/" "INBOX""#;
        let folder = SimpleImapClient::parse_list_response(line).unwrap();
        assert_eq!(folder.full_path, "INBOX");
        assert_eq!(folder.name, "INBOX");
        assert_eq!(folder.delimiter, Some('/'));
        assert_eq!(folder.folder_type, FolderType::Inbox);
    }

    #[test]
    fn test_parse_list_gmail_nested() {
        let line = r#"* LIST (\HasNoChildren) "/" "[Gmail]/Sent Mail""#;
        let folder = SimpleImapClient::parse_list_response(line).unwrap();
        assert_eq!(folder.full_path, "[Gmail]/Sent Mail");
        assert_eq!(folder.name, "Sent Mail");
        assert_eq!(folder.folder_type, FolderType::Sent);
    }

    #[test]
    fn test_parse_list_outlook_sent_items() {
        let line = r#"* LIST (\HasNoChildren \Sent) "/" "Sent Items""#;
        let folder = SimpleImapClient::parse_list_response(line).unwrap();
        assert_eq!(folder.full_path, "Sent Items");
        assert_eq!(folder.name, "Sent Items");
        assert_eq!(folder.folder_type, FolderType::Sent);
    }

    #[test]
    fn test_parse_list_outlook_junk() {
        let line = r#"* LIST (\HasNoChildren \Junk) "/" "Junk Email""#;
        let folder = SimpleImapClient::parse_list_response(line).unwrap();
        assert_eq!(folder.full_path, "Junk Email");
        assert_eq!(folder.name, "Junk Email");
        assert_eq!(folder.folder_type, FolderType::Spam);
    }

    #[test]
    fn test_parse_list_noselect_skipped() {
        let line = r#"* LIST (\Noselect \HasChildren) "/" "[Gmail]""#;
        assert!(SimpleImapClient::parse_list_response(line).is_none());
    }

    #[test]
    fn test_parse_list_root_slash_skipped() {
        // Outlook can return the root delimiter as a folder
        let line = r#"* LIST (\HasNoChildren) "/" "/""#;
        assert!(SimpleImapClient::parse_list_response(line).is_none());
    }

    #[test]
    fn test_parse_list_empty_name_skipped() {
        let line = r#"* LIST (\Noselect) "/" """#;
        // \Noselect should be filtered
        assert!(SimpleImapClient::parse_list_response(line).is_none());
    }

    #[test]
    fn test_parse_list_nil_delimiter() {
        let line = r#"* LIST (\HasNoChildren) NIL "INBOX""#;
        let folder = SimpleImapClient::parse_list_response(line).unwrap();
        assert_eq!(folder.full_path, "INBOX");
        assert_eq!(folder.delimiter, None);
    }

    #[test]
    fn test_parse_list_dot_delimiter() {
        // Some servers use "." as delimiter
        let line = r#"* LIST (\HasNoChildren) "." "INBOX.Sent""#;
        let folder = SimpleImapClient::parse_list_response(line).unwrap();
        assert_eq!(folder.full_path, "INBOX.Sent");
        assert_eq!(folder.name, "Sent");
        assert_eq!(folder.delimiter, Some('.'));
    }
}
