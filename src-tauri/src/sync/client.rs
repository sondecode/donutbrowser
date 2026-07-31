use super::types::*;
use reqwest::Client;

/// Which `x-amz-meta-*` headers still need to be sent alongside a presigned PUT.
///
/// SigV4 presigners hoist `x-amz-*` into the URL's query string and sign only
/// `host` (visible as `X-Amz-SignedHeaders=host` plus `&x-amz-...=` pairs). An
/// `x-amz-*` header that is NOT in `X-Amz-SignedHeaders` invalidates the
/// request — S3 and R2 answer `403 SignatureDoesNotMatch` — so a header must be
/// sent only when the URL didn't already carry that key. Servers that sign
/// metadata as a real header instead still get it, because then it is absent
/// from the query string.
fn metadata_headers_for(
  presigned_url: &str,
  metadata: Option<&std::collections::HashMap<String, String>>,
) -> Vec<(String, String)> {
  let Some(metadata) = metadata else {
    return Vec::new();
  };
  let query = presigned_url
    .split_once('?')
    .map(|(_, query)| query)
    .unwrap_or_default()
    .to_ascii_lowercase();

  metadata
    .iter()
    .map(|(key, value)| (format!("x-amz-meta-{}", key.to_ascii_lowercase()), value))
    .filter(|(name, _)| !query.contains(&format!("{name}=")))
    .map(|(name, value)| (name, value.clone()))
    .collect()
}

#[derive(Clone)]
pub struct SyncClient {
  client: Client,
  base_url: String,
  token: String,
}

impl SyncClient {
  pub fn new(base_url: String, token: String) -> Self {
    Self {
      client: Client::new(),
      base_url: base_url.trim_end_matches('/').to_string(),
      token,
    }
  }

  fn url(&self, path: &str) -> String {
    format!("{}/v1/objects/{}", self.base_url, path)
  }

  pub async fn stat(&self, key: &str) -> SyncResult<StatResponse> {
    let response = self
      .client
      .post(self.url("stat"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&StatRequest {
        key: key.to_string(),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn presign_upload(
    &self,
    key: &str,
    content_type: Option<&str>,
  ) -> SyncResult<PresignUploadResponse> {
    self
      .presign_upload_with_metadata(key, content_type, None)
      .await
  }

  /// Presign an upload, asking the server to sign `metadata` into the object as
  /// `x-amz-meta-*`. The response echoes the metadata the server actually signed
  /// (empty/None on older servers); the caller must send exactly that back on
  /// the PUT via `upload_bytes_with_metadata`.
  pub async fn presign_upload_with_metadata(
    &self,
    key: &str,
    content_type: Option<&str>,
    metadata: Option<std::collections::HashMap<String, String>>,
  ) -> SyncResult<PresignUploadResponse> {
    let response = self
      .client
      .post(self.url("presign-upload"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&PresignUploadRequest {
        key: key.to_string(),
        content_type: content_type.map(|s| s.to_string()),
        expires_in: Some(3600),
        metadata,
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn presign_download(&self, key: &str) -> SyncResult<PresignDownloadResponse> {
    let response = self
      .client
      .post(self.url("presign-download"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&PresignDownloadRequest {
        key: key.to_string(),
        expires_in: Some(3600),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn delete(&self, key: &str, tombstone_key: Option<&str>) -> SyncResult<DeleteResponse> {
    let response = self
      .client
      .post(self.url("delete"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&DeleteRequest {
        key: key.to_string(),
        tombstone_key: tombstone_key.map(|s| s.to_string()),
        deleted_at: Some(chrono::Utc::now().to_rfc3339()),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  pub async fn list(&self, prefix: &str) -> SyncResult<ListResponse> {
    self.list_page(prefix, None).await
  }

  async fn list_page(
    &self,
    prefix: &str,
    continuation_token: Option<String>,
  ) -> SyncResult<ListResponse> {
    let response = self
      .client
      .post(self.url("list"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&ListRequest {
        prefix: prefix.to_string(),
        max_keys: Some(1000),
        continuation_token,
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }

  /// List all objects under a prefix, paginating through all results
  pub async fn list_all(&self, prefix: &str) -> SyncResult<Vec<ListObject>> {
    let mut all_objects = Vec::new();
    let mut continuation_token: Option<String> = None;

    loop {
      let response = self.list_page(prefix, continuation_token).await?;
      all_objects.extend(response.objects);

      if !response.is_truncated {
        break;
      }
      continuation_token = response.next_continuation_token;
      if continuation_token.is_none() {
        break;
      }
    }

    Ok(all_objects)
  }

  pub async fn upload_bytes(
    &self,
    presigned_url: &str,
    data: &[u8],
    content_type: Option<&str>,
  ) -> SyncResult<()> {
    self
      .upload_bytes_with_metadata(presigned_url, data, content_type, None)
      .await
  }

  /// PUT to a presigned URL, sending `metadata` as `x-amz-meta-*` headers. These
  /// MUST be exactly the metadata the presign signed (from
  /// `PresignUploadResponse::metadata`) or S3 rejects the request.
  pub async fn upload_bytes_with_metadata(
    &self,
    presigned_url: &str,
    data: &[u8],
    content_type: Option<&str>,
    metadata: Option<&std::collections::HashMap<String, String>>,
  ) -> SyncResult<()> {
    let mut req = self
      .client
      .put(presigned_url)
      .header("Content-Length", data.len().to_string())
      .body(data.to_vec());

    if let Some(ct) = content_type {
      req = req.header("Content-Type", ct);
    }

    for (name, value) in metadata_headers_for(presigned_url, metadata) {
      req = req.header(name, value);
    }

    let response = req
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::NetworkError(format!(
        "Upload failed with status {status}: {body}"
      )));
    }

    Ok(())
  }

  pub async fn download_bytes(&self, presigned_url: &str) -> SyncResult<Vec<u8>> {
    let response = self
      .client
      .get(presigned_url)
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
      return Err(SyncError::NetworkError(format!(
        "Download failed with status: {}",
        response.status()
      )));
    }

    response
      .bytes()
      .await
      .map(|b| b.to_vec())
      .map_err(|e| SyncError::NetworkError(e.to_string()))
  }

  pub async fn presign_upload_batch(
    &self,
    items: Vec<(String, Option<String>)>,
  ) -> SyncResult<PresignUploadBatchResponse> {
    let chunk_size = 500;
    let mut all_items = Vec::new();

    for chunk in items.chunks(chunk_size) {
      let request = PresignUploadBatchRequest {
        items: chunk
          .iter()
          .map(|(key, content_type)| PresignUploadBatchItem {
            key: key.clone(),
            content_type: content_type.clone(),
          })
          .collect(),
        expires_in: Some(3600),
      };

      let response = self
        .client
        .post(self.url("presign-upload-batch"))
        .header("Authorization", format!("Bearer {}", self.token))
        .json(&request)
        .send()
        .await
        .map_err(|e| SyncError::NetworkError(e.to_string()))?;

      if response.status().is_client_error() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(SyncError::AuthError(format!("({status}) {body}")));
      }

      let batch_response: PresignUploadBatchResponse = response
        .json()
        .await
        .map_err(|e| SyncError::SerializationError(e.to_string()))?;

      all_items.extend(batch_response.items);
    }

    Ok(PresignUploadBatchResponse { items: all_items })
  }

  pub async fn presign_download_batch(
    &self,
    keys: Vec<String>,
  ) -> SyncResult<PresignDownloadBatchResponse> {
    let chunk_size = 500;
    let mut all_items = Vec::new();

    for chunk in keys.chunks(chunk_size) {
      let request = PresignDownloadBatchRequest {
        keys: chunk.to_vec(),
        expires_in: Some(3600),
      };

      let response = self
        .client
        .post(self.url("presign-download-batch"))
        .header("Authorization", format!("Bearer {}", self.token))
        .json(&request)
        .send()
        .await
        .map_err(|e| SyncError::NetworkError(e.to_string()))?;

      if response.status().is_client_error() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(SyncError::AuthError(format!("({status}) {body}")));
      }

      let batch_response: PresignDownloadBatchResponse = response
        .json()
        .await
        .map_err(|e| SyncError::SerializationError(e.to_string()))?;

      all_items.extend(batch_response.items);
    }

    Ok(PresignDownloadBatchResponse { items: all_items })
  }

  pub async fn delete_prefix(
    &self,
    prefix: &str,
    tombstone_key: Option<&str>,
  ) -> SyncResult<DeletePrefixResponse> {
    let response = self
      .client
      .post(self.url("delete-prefix"))
      .header("Authorization", format!("Bearer {}", self.token))
      .json(&DeletePrefixRequest {
        prefix: prefix.to_string(),
        tombstone_key: tombstone_key.map(|s| s.to_string()),
        deleted_at: Some(chrono::Utc::now().to_rfc3339()),
      })
      .send()
      .await
      .map_err(|e| SyncError::NetworkError(e.to_string()))?;

    if response.status().is_client_error() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(SyncError::AuthError(format!("({status}) {body}")));
    }

    response
      .json()
      .await
      .map_err(|e| SyncError::SerializationError(e.to_string()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashMap;

  fn metadata(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
      .iter()
      .map(|(k, v)| (k.to_string(), v.to_string()))
      .collect()
  }

  /// The shape every config upload actually hit: the presigner hoisted the
  /// metadata into the query and signed only `host`, so sending it as a header
  /// too made R2 answer 403 SignatureDoesNotMatch and no proxy, group or
  /// scenario ever reached the bucket.
  #[test]
  fn hoisted_metadata_is_not_resent_as_a_header() {
    let url = "https://example.r2.cloudflarestorage.com/bucket/proxies/x.json\
               ?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-SignedHeaders=host\
               &x-amz-meta-updated-at=1785288697&X-Amz-Signature=deadbeef";
    assert!(metadata_headers_for(url, Some(&metadata(&[("updated-at", "1785288697")]))).is_empty());
  }

  #[test]
  fn metadata_signed_as_a_real_header_is_still_sent() {
    let url = "https://example.com/bucket/proxies/x.json\
               ?X-Amz-SignedHeaders=host%3Bx-amz-meta-updated-at&X-Amz-Signature=deadbeef";
    assert_eq!(
      metadata_headers_for(url, Some(&metadata(&[("updated-at", "1785288697")]))),
      vec![(
        "x-amz-meta-updated-at".to_string(),
        "1785288697".to_string()
      )]
    );
  }

  #[test]
  fn only_the_hoisted_keys_are_dropped() {
    let url = "https://example.com/b/k?x-amz-meta-updated-at=1&X-Amz-Signature=d";
    let headers =
      metadata_headers_for(url, Some(&metadata(&[("updated-at", "1"), ("other", "2")])));
    assert_eq!(
      headers,
      vec![("x-amz-meta-other".to_string(), "2".to_string())]
    );
  }

  #[test]
  fn no_metadata_and_no_query_are_both_handled() {
    assert!(metadata_headers_for("https://example.com/b/k", None).is_empty());
    assert_eq!(
      metadata_headers_for(
        "https://example.com/b/k",
        Some(&metadata(&[("updated-at", "7")]))
      ),
      vec![("x-amz-meta-updated-at".to_string(), "7".to_string())]
    );
  }
}
