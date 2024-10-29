use super::error::{ServiceError, ServiceResult};
use super::{AppState, Service};
use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    Json,
};
use jstz_proto::operation::SignedOperation;
use jstz_proto::receipt::Receipt;
use tezos_data_encoding::enc::BinWriter;
use tezos_smart_rollup::inbox::ExternalMessageFrame;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub struct OperationsService;

const OPERATIONS_TAG: &str = "Operations";

/// Inject an operation into Jstz
#[utoipa::path(
        post,
        path = "",
        tag = OPERATIONS_TAG,
        responses(
            (status = 200, description = "Operation successfully injectedd"),
            (status = 400),
            (status = 500)
        )
    )]
async fn inject(
    State(AppState { rollup_client, .. }): State<AppState>,
    Json(operation): Json<SignedOperation>,
) -> ServiceResult<()> {
    let encoded_operation = bincode::serialize(&operation)
        .map_err(|_| anyhow!("Failed to serialize operation"))?;
    let address = rollup_client.get_rollup_address().await?;
    let message_frame = ExternalMessageFrame::Targetted {
        address,
        contents: encoded_operation,
    };
    let mut binary_contents = Vec::new();
    message_frame
        .bin_write(&mut binary_contents)
        .map_err(|_| anyhow!("Failed to write binary frame"))?;
    rollup_client.batcher_injection([binary_contents]).await?;
    Ok(())
}

/// Get the receipt of an operation
#[utoipa::path(
        get,
        path = "/{operation_hash}/receipt",
        tag = OPERATIONS_TAG,
        params(
            ("operation_hash" = String, description = "Operation hash")
        ),
        responses(
            (status = 200, body = Receipt),
            (status = 400),
            (status = 500)
        )
    )]
async fn receipt(
    State(AppState { rollup_client, .. }): State<AppState>,
    Path(hash): Path<String>,
) -> ServiceResult<Json<Receipt>> {
    let key = format!("/jstz_receipt/{}", hash);

    let value = rollup_client.get_value(&key).await?;

    let receipt = match value {
        Some(value) => bincode::deserialize::<Receipt>(&value)
            .map_err(|_| anyhow!("Failed to deserialize receipt"))?,
        None => Err(ServiceError::NotFound)?,
    };

    Ok(Json(receipt))
}

impl Service for OperationsService {
    fn router_with_openapi() -> OpenApiRouter<AppState> {
        let routes = OpenApiRouter::new()
            .routes(routes!(inject))
            .routes(routes!(receipt));

        OpenApiRouter::new().nest("/operations", routes)
    }
}
