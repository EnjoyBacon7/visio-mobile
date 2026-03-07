use serde::{Deserialize, Serialize};

use crate::auth::AuthService;
use crate::errors::VisioError;

/// A user returned by the Meet user search API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserSearchResult {
    pub id: String,
    pub email: String,
    pub full_name: Option<String>,
    pub short_name: Option<String>,
}

/// A resource access entry (user linked to a room with a role).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoomAccess {
    pub id: String,
    pub user: UserSearchResult,
    pub resource: String,
    pub role: String,
}

/// Paginated response wrapper from Meet API.
#[derive(Debug, Deserialize)]
struct PaginatedResponse<T> {
    results: Vec<T>,
}

/// Service for managing room access (restricted rooms).
pub struct AccessService;

impl AccessService {
    /// Search users by email (trigram similarity).
    pub async fn search_users(
        meet_url: &str,
        session_cookie: &str,
        query: &str,
    ) -> Result<Vec<UserSearchResult>, VisioError> {
        let (instance, _slug) = AuthService::parse_meet_url(meet_url)?;

        let api_url = format!(
            "https://{}/api/v1.0/users/?q={}",
            instance,
            urlencoding::encode(query)
        );

        let client = reqwest::Client::new();
        let resp = client
            .get(&api_url)
            .header(
                reqwest::header::COOKIE,
                format!("sessionid={}", session_cookie),
            )
            .send()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(VisioError::Auth(format!(
                "user search returned status {}",
                resp.status()
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        let page: PaginatedResponse<UserSearchResult> = serde_json::from_str(&body)
            .map_err(|e| VisioError::Auth(format!("invalid user search response: {e}")))?;

        Ok(page.results)
    }

    /// List accesses for a room.
    pub async fn list_accesses(
        meet_url: &str,
        session_cookie: &str,
        room_id: &str,
    ) -> Result<Vec<RoomAccess>, VisioError> {
        let (instance, _slug) = AuthService::parse_meet_url(meet_url)?;

        let api_url = format!(
            "https://{}/api/v1.0/resource-accesses/?resource={}",
            instance,
            urlencoding::encode(room_id)
        );

        let client = reqwest::Client::new();
        let resp = client
            .get(&api_url)
            .header(
                reqwest::header::COOKIE,
                format!("sessionid={}", session_cookie),
            )
            .send()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(VisioError::Auth(format!(
                "list-accesses returned status {}",
                resp.status()
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        let page: PaginatedResponse<RoomAccess> = serde_json::from_str(&body)
            .map_err(|e| VisioError::Auth(format!("invalid accesses response: {e}")))?;

        Ok(page.results)
    }

    /// Add a user as member of a room.
    pub async fn add_access(
        meet_url: &str,
        session_cookie: &str,
        user_id: &str,
        room_id: &str,
    ) -> Result<RoomAccess, VisioError> {
        use rand::Rng;

        let (instance, _slug) = AuthService::parse_meet_url(meet_url)?;

        let api_url = format!("https://{}/api/v1.0/resource-accesses/", instance);

        let csrf_bytes: [u8; 32] = rand::thread_rng().r#gen();
        let csrf_token: String = csrf_bytes.iter().map(|b| format!("{:02x}", b)).collect();

        let cookie_header = format!(
            "sessionid={}; csrftoken={}",
            session_cookie, csrf_token
        );

        let body = serde_json::json!({
            "user": user_id,
            "resource": room_id,
            "role": "member",
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&api_url)
            .header(reqwest::header::COOKIE, &cookie_header)
            .header("X-CSRFToken", &csrf_token)
            .header("Referer", format!("https://{}/", instance))
            .json(&body)
            .send()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(VisioError::Session("Already invited".to_string()));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(VisioError::Auth(format!(
                "add-access returned status {}: {}",
                status, body
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        serde_json::from_str(&body)
            .map_err(|e| VisioError::Auth(format!("invalid add-access response: {e}")))
    }

    /// Remove an access (revoke membership).
    pub async fn remove_access(
        meet_url: &str,
        session_cookie: &str,
        access_id: &str,
    ) -> Result<(), VisioError> {
        use rand::Rng;

        let (instance, _slug) = AuthService::parse_meet_url(meet_url)?;

        let api_url = format!(
            "https://{}/api/v1.0/resource-accesses/{}/",
            instance, access_id
        );

        let csrf_bytes: [u8; 32] = rand::thread_rng().r#gen();
        let csrf_token: String = csrf_bytes.iter().map(|b| format!("{:02x}", b)).collect();

        let cookie_header = format!(
            "sessionid={}; csrftoken={}",
            session_cookie, csrf_token
        );

        let client = reqwest::Client::new();
        let resp = client
            .delete(&api_url)
            .header(reqwest::header::COOKIE, &cookie_header)
            .header("X-CSRFToken", &csrf_token)
            .header("Referer", format!("https://{}/", instance))
            .send()
            .await
            .map_err(|e| VisioError::Http(e.to_string()))?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NO_CONTENT {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(VisioError::Auth(format!(
                "remove-access returned status {}: {}",
                status, body
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_user_search_result() {
        let json = r#"{"id": "abc-123", "email": "alice@example.com", "full_name": "Alice Doe", "short_name": "Alice"}"#;
        let user: UserSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(user.id, "abc-123");
        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.full_name, Some("Alice Doe".to_string()));
    }

    #[test]
    fn parse_user_search_result_minimal() {
        let json = r#"{"id": "abc-123", "email": "alice@example.com"}"#;
        let user: UserSearchResult = serde_json::from_str(json).unwrap();
        assert!(user.full_name.is_none());
        assert!(user.short_name.is_none());
    }

    #[test]
    fn parse_room_access() {
        let json = r#"{
            "id": "ra-1",
            "user": {"id": "u-1", "email": "bob@example.com", "full_name": "Bob", "short_name": null},
            "resource": "room-123",
            "role": "member"
        }"#;
        let access: RoomAccess = serde_json::from_str(json).unwrap();
        assert_eq!(access.id, "ra-1");
        assert_eq!(access.user.email, "bob@example.com");
        assert_eq!(access.role, "member");
    }

    #[test]
    fn parse_paginated_users() {
        let json = r#"{"count": 2, "next": null, "previous": null, "results": [
            {"id": "u-1", "email": "a@b.com", "full_name": "A", "short_name": null},
            {"id": "u-2", "email": "c@d.com", "full_name": "C", "short_name": null}
        ]}"#;
        let page: PaginatedResponse<UserSearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(page.results.len(), 2);
    }

    #[test]
    fn parse_paginated_accesses() {
        let json = r#"{"count": 1, "next": null, "previous": null, "results": [
            {"id": "ra-1", "user": {"id": "u-1", "email": "a@b.com", "full_name": null, "short_name": null}, "resource": "room-id", "role": "owner"}
        ]}"#;
        let page: PaginatedResponse<RoomAccess> = serde_json::from_str(json).unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].role, "owner");
    }

    #[tokio::test]
    async fn search_users_invalid_url_returns_error() {
        assert!(AccessService::search_users("invalid", "cookie", "query").await.is_err());
    }
}
