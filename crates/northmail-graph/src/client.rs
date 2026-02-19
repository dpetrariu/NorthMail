use crate::error::{GraphError, GraphResult};
use crate::types::*;
use tracing::{debug, info};

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Message fields to select in list queries (keeps payload small)
const MESSAGE_SELECT: &str = "id,internetMessageId,subject,from,toRecipients,ccRecipients,receivedDateTime,isRead,isDraft,hasAttachments,bodyPreview,flag,inferenceClassification";

pub struct GraphMailClient {
    client: reqwest::Client,
    access_token: String,
}

impl GraphMailClient {
    pub fn new(access_token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            access_token,
        }
    }

    /// List all mail folders (including child folders recursively)
    pub async fn list_folders(&self) -> GraphResult<Vec<GraphFolder>> {
        let url = format!("{}/me/mailFolders?$top=100", GRAPH_BASE);
        debug!("Graph: listing folders");

        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let list: GraphListResponse<GraphFolder> = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        let mut all_folders = Vec::new();

        // Recursively fetch child folders, building full paths
        for mut folder in list.value {
            let has_children = folder.child_folder_count > 0;
            let parent_path = folder.display_name.clone();
            folder.full_path = Some(parent_path.clone());
            all_folders.push(folder.clone());
            if has_children {
                self.list_child_folders_recursive(&folder.id, &parent_path, &mut all_folders).await?;
            }
        }

        info!("Graph: found {} folders (including children)", all_folders.len());
        Ok(all_folders)
    }

    /// Recursively list child folders, building hierarchical paths
    async fn list_child_folders_recursive(
        &self,
        parent_id: &str,
        parent_path: &str,
        result: &mut Vec<GraphFolder>,
    ) -> GraphResult<()> {
        let url = format!(
            "{}/me/mailFolders/{}/childFolders?$top=100",
            GRAPH_BASE, parent_id
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let list: GraphListResponse<GraphFolder> = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        for mut folder in list.value {
            let has_children = folder.child_folder_count > 0;
            let folder_path = format!("{}/{}", parent_path, folder.display_name);
            folder.full_path = Some(folder_path.clone());
            result.push(folder.clone());
            if has_children {
                Box::pin(self.list_child_folders_recursive(&folder.id, &folder_path, result)).await?;
            }
        }

        Ok(())
    }

    /// List messages in a folder with pagination
    pub async fn list_messages(
        &self,
        folder_id: &str,
        top: u32,
        skip: u32,
    ) -> GraphResult<(Vec<GraphMessageEnvelope>, Option<String>)> {
        let url = format!(
            "{}/me/mailFolders/{}/messages?$select={}&$top={}&$skip={}&$orderby=receivedDateTime desc",
            GRAPH_BASE, folder_id, MESSAGE_SELECT, top, skip
        );
        debug!("Graph: listing messages folder={} top={} skip={}", folder_id, top, skip);

        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let list: GraphListResponse<GraphMessageEnvelope> = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        let next_link = list.next_link;
        debug!("Graph: got {} messages, has_more={}", list.value.len(), next_link.is_some());
        Ok((list.value, next_link))
    }

    /// List messages using a next_link URL (for pagination)
    pub async fn list_messages_next(
        &self,
        next_link: &str,
    ) -> GraphResult<(Vec<GraphMessageEnvelope>, Option<String>)> {
        debug!("Graph: fetching next page");

        let response = self
            .client
            .get(next_link)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let list: GraphListResponse<GraphMessageEnvelope> = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        let next_link = list.next_link;
        Ok((list.value, next_link))
    }

    /// Fetch raw MIME (RFC 2822) body of a message
    pub async fn fetch_mime_body(&self, message_id: &str) -> GraphResult<String> {
        let url = format!("{}/me/messages/{}/$value", GRAPH_BASE, message_id);
        debug!("Graph: fetching MIME body for {}", message_id);

        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let body = response.text().await?;
        debug!("Graph: got MIME body {} bytes", body.len());
        Ok(body)
    }

    /// Set read/unread status
    pub async fn set_read(&self, message_id: &str, is_read: bool) -> GraphResult<()> {
        let url = format!("{}/me/messages/{}", GRAPH_BASE, message_id);
        debug!("Graph: setting isRead={} for {}", is_read, message_id);

        let response = self
            .client
            .patch(&url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "isRead": is_read }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        Ok(())
    }

    /// Set flagged/unflagged status
    pub async fn set_flagged(&self, message_id: &str, flagged: bool) -> GraphResult<()> {
        let url = format!("{}/me/messages/{}", GRAPH_BASE, message_id);
        let flag_status = if flagged { "flagged" } else { "notFlagged" };
        debug!("Graph: setting flag={} for {}", flag_status, message_id);

        let response = self
            .client
            .patch(&url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({
                "flag": { "flagStatus": flag_status }
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        Ok(())
    }

    /// Move a message to a different folder. Returns the new message ID.
    pub async fn move_message(
        &self,
        message_id: &str,
        dest_folder_id: &str,
    ) -> GraphResult<String> {
        let url = format!("{}/me/messages/{}/move", GRAPH_BASE, message_id);
        debug!("Graph: moving {} to {}", message_id, dest_folder_id);

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&MoveRequest {
                destination_id: dest_folder_id.to_string(),
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let moved: MoveResponse = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        info!("Graph: moved message, new id={}", moved.id);
        Ok(moved.id)
    }

    /// Create a draft message in the Drafts folder from message fields directly.
    /// Returns the Graph message ID of the created draft.
    pub async fn create_draft_from_message(
        &self,
        subject: &str,
        body_text: &str,
        body_html: Option<&str>,
        to: &[String],
        cc: &[String],
        attachments: &[(String, String, Vec<u8>)], // (filename, mime_type, data)
    ) -> GraphResult<String> {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;

        let to_recipients: Vec<serde_json::Value> = to.iter()
            .filter(|addr| !addr.is_empty())
            .map(|addr| serde_json::json!({
                "emailAddress": { "address": addr }
            }))
            .collect();

        let cc_recipients: Vec<serde_json::Value> = cc.iter()
            .filter(|addr| !addr.is_empty())
            .map(|addr| serde_json::json!({
                "emailAddress": { "address": addr }
            }))
            .collect();

        let (content_type, content) = match body_html {
            Some(html) => ("HTML", html),
            None => ("Text", body_text),
        };

        let mut draft = serde_json::json!({
            "subject": subject,
            "body": {
                "contentType": content_type,
                "content": content,
            },
        });

        if !to_recipients.is_empty() {
            draft["toRecipients"] = serde_json::Value::Array(to_recipients);
        }
        if !cc_recipients.is_empty() {
            draft["ccRecipients"] = serde_json::Value::Array(cc_recipients);
        }
        if !attachments.is_empty() {
            let graph_attachments: Vec<serde_json::Value> = attachments.iter()
                .map(|(filename, mime_type, data)| serde_json::json!({
                    "@odata.type": "#microsoft.graph.fileAttachment",
                    "name": filename,
                    "contentType": mime_type,
                    "contentBytes": engine.encode(data),
                }))
                .collect();
            draft["attachments"] = serde_json::Value::Array(graph_attachments);
        }

        let url = format!("{}/me/messages", GRAPH_BASE);
        debug!("Graph: creating draft, subject={}, attachments={}", subject, attachments.len());

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&draft)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let created: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        let id = created["id"]
            .as_str()
            .ok_or_else(|| GraphError::ParseError("No id in draft response".to_string()))?
            .to_string();

        info!("Graph: created draft, id={}", id);
        Ok(id)
    }

    /// Fetch all file attachments for a message with their actual data.
    /// Returns Vec<(filename, mime_type, data_bytes)>.
    pub async fn list_attachments(
        &self,
        message_id: &str,
    ) -> GraphResult<Vec<(String, String, Vec<u8>)>> {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;

        let url = format!(
            "{}/me/messages/{}/attachments?$filter=isInline eq false",
            GRAPH_BASE, message_id
        );
        debug!("Graph: listing attachments for {}", message_id);

        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let list: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        let mut result = Vec::new();
        if let Some(items) = list["value"].as_array() {
            for item in items {
                // Only include fileAttachment types (skip referenceAttachment, itemAttachment)
                let odata_type = item["@odata.type"].as_str().unwrap_or("");
                if odata_type != "#microsoft.graph.fileAttachment" {
                    continue;
                }

                let name = item["name"].as_str().unwrap_or("attachment").to_string();
                let content_type = item["contentType"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let content_bytes = item["contentBytes"]
                    .as_str()
                    .unwrap_or("");

                let data = engine.decode(content_bytes).unwrap_or_default();
                info!("Graph: attachment '{}' ({}) {} bytes", name, content_type, data.len());
                result.push((name, content_type, data));
            }
        }

        info!("Graph: found {} file attachments for {}", result.len(), message_id);
        Ok(result)
    }

    /// Update an existing draft message (PATCH - preserves attachments).
    /// Only updates subject, body, and recipients. Does NOT touch attachments.
    pub async fn update_draft(
        &self,
        message_id: &str,
        subject: &str,
        body_text: &str,
        body_html: Option<&str>,
        to: &[String],
        cc: &[String],
    ) -> GraphResult<()> {
        let to_recipients: Vec<serde_json::Value> = to.iter()
            .filter(|addr| !addr.is_empty())
            .map(|addr| serde_json::json!({
                "emailAddress": { "address": addr }
            }))
            .collect();

        let cc_recipients: Vec<serde_json::Value> = cc.iter()
            .filter(|addr| !addr.is_empty())
            .map(|addr| serde_json::json!({
                "emailAddress": { "address": addr }
            }))
            .collect();

        let (content_type, content) = match body_html {
            Some(html) => ("HTML", html),
            None => ("Text", body_text),
        };

        let mut patch = serde_json::json!({
            "subject": subject,
            "body": {
                "contentType": content_type,
                "content": content,
            },
            "toRecipients": to_recipients,
            "ccRecipients": cc_recipients,
        });

        // If no recipients, set empty arrays (don't omit, otherwise server keeps old values)
        if to_recipients.is_empty() {
            patch["toRecipients"] = serde_json::Value::Array(vec![]);
        }
        if cc_recipients.is_empty() {
            patch["ccRecipients"] = serde_json::Value::Array(vec![]);
        }

        let url = format!("{}/me/messages/{}", GRAPH_BASE, message_id);
        debug!("Graph: updating draft {}", message_id);

        let response = self
            .client
            .patch(&url)
            .bearer_auth(&self.access_token)
            .json(&patch)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        info!("Graph: updated draft {}", message_id);
        Ok(())
    }

    /// Create a new mail folder. If parent_folder_id is provided, creates a child folder.
    /// Returns the new folder's Graph ID.
    pub async fn create_folder(
        &self,
        display_name: &str,
        parent_folder_id: Option<&str>,
    ) -> GraphResult<String> {
        let url = match parent_folder_id {
            Some(parent_id) => format!(
                "{}/me/mailFolders/{}/childFolders",
                GRAPH_BASE, parent_id
            ),
            None => format!("{}/me/mailFolders", GRAPH_BASE),
        };
        debug!("Graph: creating folder '{}' parent={:?}", display_name, parent_folder_id);

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "displayName": display_name }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        let created: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GraphError::ParseError(e.to_string()))?;

        let id = created["id"]
            .as_str()
            .ok_or_else(|| GraphError::ParseError("No id in create folder response".to_string()))?
            .to_string();

        info!("Graph: created folder '{}', id={}", display_name, id);
        Ok(id)
    }

    /// Rename a mail folder
    pub async fn rename_folder(
        &self,
        folder_id: &str,
        new_name: &str,
    ) -> GraphResult<()> {
        let url = format!("{}/me/mailFolders/{}", GRAPH_BASE, folder_id);
        debug!("Graph: renaming folder {} to '{}'", folder_id, new_name);

        let response = self
            .client
            .patch(&url)
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "displayName": new_name }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        info!("Graph: renamed folder {} to '{}'", folder_id, new_name);
        Ok(())
    }

    /// Delete a mail folder
    pub async fn delete_folder(&self, folder_id: &str) -> GraphResult<()> {
        let url = format!("{}/me/mailFolders/{}", GRAPH_BASE, folder_id);
        debug!("Graph: deleting folder {}", folder_id);

        let response = self
            .client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        info!("Graph: deleted folder {}", folder_id);
        Ok(())
    }

    /// Empty a folder by deleting all messages in it
    pub async fn empty_folder(&self, folder_id: &str) -> GraphResult<u64> {
        let mut deleted = 0u64;
        loop {
            // Fetch a batch of message IDs (only need the id field)
            let url = format!(
                "{}/me/mailFolders/{}/messages?$select=id&$top=100",
                GRAPH_BASE, folder_id
            );

            let response = self
                .client
                .get(&url)
                .bearer_auth(&self.access_token)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(GraphError::ApiError { status, body });
            }

            let list: serde_json::Value = response
                .json()
                .await
                .map_err(|e| GraphError::ParseError(e.to_string()))?;

            let ids: Vec<String> = list["value"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if ids.is_empty() {
                break;
            }

            for id in &ids {
                self.delete_message(id).await?;
                deleted += 1;
            }

            debug!("Graph: deleted {} messages so far from folder {}", deleted, folder_id);
        }

        info!("Graph: emptied folder {}, deleted {} messages", folder_id, deleted);
        Ok(deleted)
    }

    /// Delete a message permanently
    pub async fn delete_message(&self, message_id: &str) -> GraphResult<()> {
        let url = format!("{}/me/messages/{}", GRAPH_BASE, message_id);
        debug!("Graph: deleting {}", message_id);

        let response = self
            .client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(GraphError::ApiError { status, body });
        }

        Ok(())
    }
}
