//! SMTP client implementation

use crate::{SmtpError, SmtpResult};
use lettre::{
    message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::{Credentials, Mechanism},
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing::info;

/// An attachment to include in an outgoing message
#[derive(Debug, Clone)]
pub struct OutgoingAttachment {
    /// Filename to display
    pub filename: String,
    /// MIME type (e.g., "application/pdf")
    pub mime_type: String,
    /// Raw file data
    pub data: Vec<u8>,
}

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
    /// File attachments
    pub attachments: Vec<OutgoingAttachment>,
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
            attachments: Vec::new(),
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

    /// Add an attachment
    pub fn attachment(mut self, filename: impl Into<String>, mime_type: impl Into<String>, data: Vec<u8>) -> Self {
        self.attachments.push(OutgoingAttachment {
            filename: filename.into(),
            mime_type: mime_type.into(),
            data,
        });
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

    /// Create an Outlook SMTP client
    pub fn outlook() -> Self {
        Self::new("smtp.office365.com", 587)
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

        // lettre's Xoauth2 mechanism expects the access token directly -
        // it constructs and encodes the XOAUTH2 string internally
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)
            .map_err(|e| SmtpError::ConnectionFailed(e.to_string()))?
            .port(self.port)
            .credentials(Credentials::new(email.to_string(), access_token.to_string()))
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

    /// Send a message using password authentication (PLAIN mechanism)
    pub async fn send_password(
        &self,
        email: &str,
        password: &str,
        message: OutgoingMessage,
    ) -> SmtpResult<()> {
        info!("Sending email via SMTP with password auth");

        let lettre_message = self.build_message(&message)?;

        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)
            .map_err(|e| SmtpError::ConnectionFailed(e.to_string()))?
            .port(self.port)
            .credentials(Credentials::new(email.to_string(), password.to_string()))
            .authentication(vec![Mechanism::Plain])
            .build();

        transport
            .send(lettre_message)
            .await
            .map_err(|e| SmtpError::SendFailed(e.to_string()))?;

        info!("Email sent successfully");
        Ok(())
    }

    /// Build a lettre Message from OutgoingMessage
    fn build_message(&self, msg: &OutgoingMessage) -> SmtpResult<Message> {
        build_lettre_message(msg)
    }
}

/// Build a lettre Message from OutgoingMessage (standalone, no SmtpClient needed)
pub fn build_lettre_message(msg: &OutgoingMessage) -> SmtpResult<Message> {
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

    // Build the body part (text/html or multipart/alternative)
    let body_part = match (&msg.text_body, &msg.html_body) {
        (Some(text), Some(html)) => {
            // Multipart alternative for both text and HTML
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
                )
        }
        (Some(text), None) => MultiPart::alternative().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(text.clone()),
        ),
        (None, Some(html)) => MultiPart::alternative().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(html.clone()),
        ),
        (None, None) => MultiPart::alternative().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(String::new()),
        ),
    };

    // If there are attachments, wrap in multipart/mixed
    let message = if msg.attachments.is_empty() {
        builder
            .multipart(body_part)
            .map_err(|e| SmtpError::MessageBuildError(e.to_string()))?
    } else {
        let mut mixed = MultiPart::mixed().multipart(body_part);

        for att in &msg.attachments {
            // Parse MIME type or default to application/octet-stream
            let content_type = att
                .mime_type
                .parse::<ContentType>()
                .unwrap_or(ContentType::parse("application/octet-stream").unwrap());

            let attachment = Attachment::new(att.filename.clone()).body(att.data.clone(), content_type);
            mixed = mixed.singlepart(attachment);
        }

        builder
            .multipart(mixed)
            .map_err(|e| SmtpError::MessageBuildError(e.to_string()))?
    };

    Ok(message)
}
