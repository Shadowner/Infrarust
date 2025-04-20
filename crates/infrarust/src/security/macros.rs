//! Macros for working with filters
/// Executes an operation on a filter of a specific type
///
/// # Arguments
/// * `registry` - The filter registry to get the filter from
/// * `filter_name` - The name of the filter
/// * `filter_type` - The expected type of the filter
/// * `operation` - A closure that takes a reference to the typed filter and returns a Result
///
/// # Returns
/// The result of the operation or an error if the filter doesn't exist or is not of the expected type
#[macro_export]
macro_rules! with_filter {
    ($registry:expr, $filter_name:expr, $filter_type:ty, $operation:expr) => {
        match $registry.get_filter($filter_name).await {
            Ok(filter) => {
                let filter_any = filter.as_any();
                if let Some(typed_filter) = filter_any.downcast_ref::<$filter_type>() {
                    $operation(typed_filter).await
                } else {
                    Err(FilterError::Other(format!(
                        "Found filter '{}' but it is not a {}",
                        $filter_name,
                        stringify!($filter_type)
                    )))
                }
            }
            Err(err) => Err(err),
        }
    };
}

/// For operations returning Result<T, FilterError>
#[macro_export]
macro_rules! with_filter_result {
    ($registry:expr, $filter_name:expr, $filter_type:ty, $operation:expr, $default:expr) => {{
        match $registry.get_filter($filter_name).await {
            Ok(filter_ref) => {
                let filter_any = filter_ref.as_any();
                if let Some(typed_filter) = filter_any.downcast_ref::<$filter_type>() {
                    $operation(typed_filter).await
                } else {
                    Err(FilterError::Other(format!(
                        "Found filter '{}' but it is not a {}",
                        $filter_name,
                        stringify!($filter_type)
                    )))
                }
            }
            Err(FilterError::NotFound(_)) => Ok($default),
            Err(e) => Err(e),
        }
    }};
}

/// For operations that don't return Result (void/unit operations)
#[macro_export]
macro_rules! with_filter_void {
    ($registry:expr, $filter_name:expr, $filter_type:ty, $operation:expr) => {{
        match $registry.get_filter($filter_name).await {
            Ok(filter_ref) => {
                let filter_any = filter_ref.as_any();
                if let Some(typed_filter) = filter_any.downcast_ref::<$filter_type>() {
                    $operation(typed_filter).await;
                } else {
                    tracing::warn!(
                        "Found filter '{}' but it is not a {}",
                        $filter_name,
                        stringify!($filter_type)
                    );
                }
            }
            Err(FilterError::NotFound(_)) => {
                tracing::debug!("Filter '{}' not found", $filter_name);
            }
            Err(e) => {
                tracing::warn!("Error getting filter '{}': {:?}", $filter_name, e);
            }
        }
    }};
}

/// Like `with_filter`, but returns a default value if the filter is not found
///
/// # Arguments
/// * `registry` - The filter registry to get the filter from
/// * `filter_name` - The name of the filter
/// * `filter_type` - The expected type of the filter
/// * `operation` - A closure that takes a reference to the typed filter and returns a Result
/// * `default` - The default value to return if the filter is not found
///
/// # Returns
/// The result of the operation, the default value if the filter doesn't exist,
/// or an error if the filter exists but is not of the expected type
#[macro_export]
macro_rules! with_filter_or {
    ($registry:expr, $filter_name:expr, $filter_type:ty, $operation:expr, $default:expr) => {{ $crate::with_filter_result!($registry, $filter_name, $filter_type, $operation, $default) }};
}
