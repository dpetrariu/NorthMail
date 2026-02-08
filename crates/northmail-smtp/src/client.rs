//! SMTP client implementation

use crate::{SmtpError, SmtpResult};
use lettre::{
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::{Credentials, Mechanism},
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing::info;

/// Email message to send
#[derive(Debug, Clone)]
pub struct OutgoingMessage {
    /// From address
    pub from: String,
    /// From display name
    pub from_name: Option<String>,
    /// To addresses
    pub to: Vec<String>,
    /// CC addresses
    pub cc: Vec<String>,
    /// BCC addresses
    pub bcc: Vec<String>,
    /// Subject line
    pub subject: String,
    /// Plain text body
    pub text_body: Option<String>,
    /// HTML body
    pub html_body: Option<String>,
    /// In-Reply-To header
    pub in_reply_to: Option<String>,
    /// References header
    pub references: Vec<String>,
}

impl OutgoingMessage {
    /// Create a new message builder
    pub fn new(from: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            from_name: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.into(),
            text_body: None,
            html_body: None,
            in_reply_to: None,
            references: Vec::new(),
        }
    }

    /// Set the from display name
    pub fn from_name(mut self, name: impl Into<String>) -> Self {
        self.from_name = Some(name.into());
        self
    }

    /// Add a To recipient
    pub fn to(mut self, address: impl Into<String>) -> Self {
        self.to.push(address.into());
        self
    }

    /// Add a CC recipient
    pub fn cc(mut self, address: impl Into<String>) -> Self {
        self.cc.push(address.into());
        self
    }

    /// Add a BCC recipient
    pub fn bcc(mut self, address: impl Into<String>) -> Self {
        self.bcc.push(address.into());
        self
    }

    /// Set the plain text body
    pub fn text(mut self, body: impl Into<String>) -> Self {
        self.text_body = Some(body.into());
        self
    }

    /// Set the HTML body
    pub fn html(mut self, body: impl Into<String>) -> Self {
        self.html_body = Some(body.into());
        self
    }

    /// Set the In-Reply-To header
    pub fn reply_to_message(mut self, message_id: impl Into<String>) -> Self {
        self.in_reply_to = Some(message_id.into());
        self
    }

    /// Add a reference
    pub fn reference(mut self, message_id: impl Into<String>) -> Self {
        self.references.push(message_id.into());
        self
    }
}

/// SMTP client for sending emails
pub struct SmtpClient {
    host: String,
    port: u16,
}

impl SmtpClient {
    /// Create a new SMTP client
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// Create a Gmail SMTP client
    pub fn gmail() -> Self {
        Self::new("smtp.gmail.com", 587)
    }

    /// Send a message using XOAUTH2 authentication
    pub async fn send_xoauth2(
        &self,
        email: &str,
        access_token: &str,
        message: OutgoingMessage,
    ) -> SmtpResult<()> {
        info!("Sending email via SMTP with XOAUTH2");

        // Build the lettre message
        let lettre_message = self.build_message(&message)?;

        // Create XOAUTH2 credentials
        // lettre expects the XOAUTH2 string to be passed as the password
        let xoauth2_string = format!("user={}\x01auth=Bearer {}\x01\x01", email, access_token);

        // Build SMTP transport with XOAUTH2
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)
            .map_err(|e| SmtpError::ConnectionFailed(e.to_string()))?
            .port(self.port)
            .credentials(Credentials::new(email.to_string(), xoauth2_string))
            .authentication(vec![Mechanism::Xoauth2])
            .build();

        // Send the message
        transport
            .send(lettre_message)
            .await
            .map_err(|e| SmtpError::SendFailed(e.to_string()))?;

        info!("Email sent successfully");
        Ok(())
    }

    /// Build a lettre Message from OutgoingMessage
    fn build_message(&self, msg: &OutgoingMessage) -> SmtpResult<Message> {
        // Parse from address
        let from_mailbox = if let Some(ref name) = msg.from_name {
            Mailbox::new(
                Some(name.clone()),
                msg.from
                    .parse()
                    .map_err(|e| SmtpError::InvalidAddress(format!("{}: {}", msg.from, e)))?,
            )
        } else {
            Mailbox::new(
                None,
                msg.from
                    .parse()
                    .map_err(|e| SmtpError::InvalidAddress(format!("{}: {}", msg.from, e)))?,
            )
        };

        let mut builder = Message::builder().from(from_mailbox).subject(&msg.subject);

        // Add To recipients
        for to in &msg.to {
            let mailbox = Mailbox::new(
                None,
                to.parse()
                    .map_err(|e| SmtpError::InvalidAddress(format!("{}: {}", to, e)))?,
            );
            builder = builder.to(mailbox);
        }

        // Add CC recipients
        for cc in &msg.cc {
            let mailbox = Mailbox::new(
                None,
                cc.parse()
                    .map_err(|e| SmtpError::InvalidAddress(format!("{}: {}", cc, e)))?,
            );
            builder = builder.cc(mailbox);
        }

        // Add BCC recipients
        for bcc in &msg.bcc {
            let mailbox = Mailbox::new(
                None,
                bcc.parse()
                    .map_err(|e| SmtpError::InvalidAddress(format!("{}: {}", bcc, e)))?,
            );
            builder = builder.bcc(mailbox);
        }

        // Add In-Reply-To header if present
        if let Some(ref reply_to) = msg.in_reply_to {
            builder = builder.in_reply_to(reply_to.clone());
        }

        // Add References header if present
        if !msg.references.is_empty() {
            builder = builder.references(msg.references.join(" "));
        }

        // Build body
        let message = match (&msg.text_body, &msg.html_body) {
            (Some(text), Some(html)) => {
                // Multipart alternative for both text and HTML
                builder
                    .multipart(
                        MultiPart::alternative()
                            .singlepart(
                                SinglePart::builder()
                                    .header(ContentType::TEXT_PLAIN)
                                    .body(text.clone()),
                            )
                            .singlepart(
                                SinglePart::builder()
                                    .header(ContentType::TEXT_HTML)
                                    .body(html.clone()),
                            ),
                    )
                    .map_err(|e| SmtpError::MessageBuildError(e.to_string()))?
            }
            (Some(text), None) => builder
                .body(text.clone())
                .map_err(|e| SmtpError::MessageBuildError(e.to_string()))?,
            (None, Some(html)) => builder
                .header(ContentType::TEXT_HTML)
                .body(html.clone())
                .map_err(|e| SmtpError::MessageBuildError(e.to_string()))?,
            (None, None) => builder
                .body(String::new())
                .map_err(|e| SmtpError::MessageBuildError(e.to_string()))?,
        };

        Ok(message)
    }
}
