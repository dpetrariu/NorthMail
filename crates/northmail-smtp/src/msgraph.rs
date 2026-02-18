//! Microsoft Graph API email sending
//!
//! Sends emails via POST /me/sendMail using the Graph API.
//! This works with GOA OAuth2 tokens which have `mail.send` scope.

use crate::{OutgoingMessage, SmtpError, SmtpResult};
use base64::Engine;
use serde::Serialize;
use tracing::info;

const GRAPH_SEND_MAIL_URL: &str = "https://graph.microsoft.com/v1.0/me/sendMail";

#[derive(Serialize)]
struct SendMailRequest {
    message: GraphMessage,
    #[serde(rename = "saveToSentItems")]
    save_to_sent_items: bool,
}

#[derive(Serialize)]
struct GraphMessage {
    subject: String,
    body: GraphBody,
    #[serde(rename = "toRecipients")]
    to_recipients: Vec<GraphRecipient>,
    #[serde(rename = "ccRecipients", skip_serializing_if = "Vec::is_empty")]
    cc_recipients: Vec<GraphRecipient>,
    #[serde(rename = "bccRecipients", skip_serializing_if = "Vec::is_empty")]
    bcc_recipients: Vec<GraphRecipient>,
    #[serde(rename = "internetMessageHeaders", skip_serializing_if = "Vec::is_empty")]
    internet_message_headers: Vec<GraphHeader>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<GraphAttachment>,
}

#[derive(Serialize)]
struct GraphBody {
    #[serde(rename = "contentType")]
    content_type: String,
    content: String,
}

#[derive(Serialize)]
struct GraphRecipient {
    #[serde(rename = "emailAddress")]
    email_address: GraphEmailAddress,
}

#[derive(Serialize)]
struct GraphEmailAddress {
    address: String,
}

#[derive(Serialize)]
struct GraphHeader {
    name: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphAttachment {
    #[serde(rename = "@odata.type")]
    odata_type: String,
    name: String,
    content_type: String,
    content_bytes: String,
}

/// Send an email via Microsoft Graph API
pub async fn send_via_graph(access_token: &str, message: OutgoingMessage) -> SmtpResult<()> {
    info!("Sending email via Microsoft Graph API");

    let (content_type, content) = match (&message.html_body, &message.text_body) {
        (Some(html), _) => ("HTML".to_string(), html.clone()),
        (None, Some(text)) => ("Text".to_string(), text.clone()),
        (None, None) => ("Text".to_string(), String::new()),
    };

    let to_recipients: Vec<GraphRecipient> = message
        .to
        .iter()
        .map(|addr| GraphRecipient {
            email_address: GraphEmailAddress {
                address: addr.clone(),
            },
        })
        .collect();

    let cc_recipients: Vec<GraphRecipient> = message
        .cc
        .iter()
        .map(|addr| GraphRecipient {
            email_address: GraphEmailAddress {
                address: addr.clone(),
            },
        })
        .collect();

    let bcc_recipients: Vec<GraphRecipient> = message
        .bcc
        .iter()
        .map(|addr| GraphRecipient {
            email_address: GraphEmailAddress {
                address: addr.clone(),
            },
        })
        .collect();

    // Add In-Reply-To and References as internet message headers
    let mut headers = Vec::new();
    if let Some(ref reply_to) = message.in_reply_to {
        headers.push(GraphHeader {
            name: "In-Reply-To".to_string(),
            value: reply_to.clone(),
        });
    }
    if !message.references.is_empty() {
        headers.push(GraphHeader {
            name: "References".to_string(),
            value: message.references.join(" "),
        });
    }

    // Convert attachments
    let engine = base64::engine::general_purpose::STANDARD;
    let attachments: Vec<GraphAttachment> = message
        .attachments
        .iter()
        .map(|att| GraphAttachment {
            odata_type: "#microsoft.graph.fileAttachment".to_string(),
            name: att.filename.clone(),
            content_type: att.mime_type.clone(),
            content_bytes: engine.encode(&att.data),
        })
        .collect();

    let request = SendMailRequest {
        message: GraphMessage {
            subject: message.subject.clone(),
            body: GraphBody {
                content_type,
                content,
            },
            to_recipients,
            cc_recipients,
            bcc_recipients,
            internet_message_headers: headers,
            attachments,
        },
        save_to_sent_items: true,
    };

    let request_json = serde_json::to_string(&request)
        .unwrap_or_else(|_| "<failed to serialize>".to_string());
    info!("Graph sendMail request body length: {} bytes", request_json.len());
    info!("Graph sendMail to: {:?}", message.to);

    let client = reqwest::Client::new();
    let response = client
        .post(GRAPH_SEND_MAIL_URL)
        .bearer_auth(access_token)
        .json(&request)
        .send()
        .await
        .map_err(|e| SmtpError::SendFailed(format!("Graph API request failed: {}", e)))?;

    let status = response.status();
    info!("Graph sendMail response status: {}", status);

    if status.is_success() {
        info!("Email sent successfully via Graph API (status {})", status);
        Ok(())
    } else {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());
        Err(SmtpError::SendFailed(format!(
            "Graph API returned {}: {}",
            status, body
        )))
    }
}
