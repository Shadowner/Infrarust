use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub data: T,
}

pub fn ok<T: Serialize>(data: T) -> Json<ApiResponse<T>> {
    Json(ApiResponse { data })
}

#[derive(Serialize)]
pub struct MutationResult {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub fn mutation_ok(message: impl Into<String>) -> Json<ApiResponse<MutationResult>> {
    ok(MutationResult {
        success: true,
        message: message.into(),
        details: None,
    })
}

pub fn created(
    data: MutationResult,
) -> (axum::http::StatusCode, Json<ApiResponse<MutationResult>>) {
    (axum::http::StatusCode::CREATED, Json(ApiResponse { data }))
}

pub(crate) const fn default_page() -> usize {
    1
}

pub(crate) const fn default_per_page() -> usize {
    20
}

const MAX_PER_PAGE: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
}

impl PaginationParams {
    pub fn normalize(&mut self) {
        if self.page == 0 {
            self.page = 1;
        }
        self.per_page = self.per_page.clamp(1, MAX_PER_PAGE);
    }

    pub fn apply<T: Serialize>(&self, items: Vec<T>) -> PaginatedResponse<T> {
        let total = items.len();
        let total_pages = if total == 0 {
            1
        } else {
            total.div_ceil(self.per_page)
        };
        let offset = (self.page - 1) * self.per_page;
        let data: Vec<T> = items.into_iter().skip(offset).take(self.per_page).collect();

        PaginatedResponse {
            data,
            meta: PaginationMeta {
                total,
                page: self.page,
                per_page: self.per_page,
                total_pages,
            },
        }
    }
}

#[derive(Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    pub meta: PaginationMeta,
}

#[derive(Serialize)]
pub struct PaginationMeta {
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
    pub total_pages: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_defaults() {
        let params: PaginationParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.page, 1);
        assert_eq!(params.per_page, 20);
    }

    #[test]
    fn pagination_normalize_clamps() {
        let mut params = PaginationParams {
            page: 0,
            per_page: 999,
        };
        params.normalize();
        assert_eq!(params.page, 1);
        assert_eq!(params.per_page, MAX_PER_PAGE);

        let mut params = PaginationParams {
            page: 1,
            per_page: 0,
        };
        params.normalize();
        assert_eq!(params.per_page, 1);
    }

    #[test]
    fn pagination_apply_basic() {
        let mut params = PaginationParams {
            page: 2,
            per_page: 3,
        };
        params.normalize();
        let items: Vec<i32> = (1..=10).collect();
        let result = params.apply(items);

        assert_eq!(result.data, vec![4, 5, 6]);
        assert_eq!(result.meta.total, 10);
        assert_eq!(result.meta.page, 2);
        assert_eq!(result.meta.per_page, 3);
        assert_eq!(result.meta.total_pages, 4);
    }

    #[test]
    fn pagination_beyond_total() {
        let mut params = PaginationParams {
            page: 99,
            per_page: 10,
        };
        params.normalize();
        let items: Vec<i32> = (1..=5).collect();
        let result = params.apply(items);

        assert!(result.data.is_empty());
        assert_eq!(result.meta.total, 5);
        assert_eq!(result.meta.total_pages, 1);
    }

    #[test]
    fn pagination_empty_items() {
        let mut params = PaginationParams {
            page: 1,
            per_page: 10,
        };
        params.normalize();
        let result = params.apply(Vec::<i32>::new());

        assert!(result.data.is_empty());
        assert_eq!(result.meta.total, 0);
        assert_eq!(result.meta.total_pages, 1);
    }
}
