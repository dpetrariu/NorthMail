use serde::{Deserialize, Serialize};

/// Response wrapper for Graph API list endpoints
#[derive(Debug, Deserialize)]
pub struct GraphListResponse<T> {
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
}

/// A mail folder from Graph API
#[derive(Debug, Clone, Deserialize)]
pub struct GraphFolder {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "totalItemCount")]
    pub total_item_count: i64,
    #[serde(rename = "unreadItemCount")]
    pub unread_item_count: i64,
}

/// A message envelope from Graph API (lightweight, no body)
#[derive(Debug, Clone, Deserialize)]
pub struct GraphMessageEnvelope {
    pub id: String,
    #[serde(rename = "internetMessageId")]
    pub internet_message_id: Option<String>,
    pub subject: Option<String>,
    pub from: Option<GraphEmailWrapper>,
    #[serde(rename = "toRecipients", default)]
    pub to_recipients: Vec<GraphEmailWrapper>,
    #[serde(rename = "ccRecipients", default)]
    pub cc_recipients: Vec<GraphEmailWrapper>,
    #[serde(rename = "receivedDateTime")]
    pub received_date_time: Option<String>,
    #[serde(rename = "isRead")]
    pub is_read: bool,
    #[serde(rename = "isDraft")]
    pub is_draft: Option<bool>,
    #[serde(rename = "hasAttachments")]
    pub has_attachments: bool,
    #[serde(rename = "bodyPreview")]
    pub body_preview: Option<String>,
    pub flag: Option<GraphFlag>,
    #[serde(rename = "inferenceClassification")]
    pub inference_classification: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphEmailWrapper {
    #[serde(rename = "emailAddress")]
    pub email_address: GraphEmailAddress,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphEmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphFlag {
    #[serde(rename = "flagStatus")]
    pub flag_status: String,
}

/// Request body for moving a message
#[derive(Debug, Serialize)]
pub struct MoveRequest {
    #[serde(rename = "destinationId")]
    pub destination_id: String,
}

/// Response from move operation
#[derive(Debug, Deserialize)]
pub struct MoveResponse {
    pub id: String,
}
