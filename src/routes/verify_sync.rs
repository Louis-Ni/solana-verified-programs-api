use crate::builder::verify_build;
use crate::db::DbClient;
use crate::models::{
    ApiResponse, ErrorResponse, JobStatus, SolanaProgramBuild, SolanaProgramBuildParams, Status,
    StatusResponse,
};
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;

pub(crate) async fn verify_sync(
    State(db): State<DbClient>,
    Json(payload): Json<SolanaProgramBuildParams>,
) -> (StatusCode, Json<ApiResponse>) {
    let verify_build_data = SolanaProgramBuild {
        id: uuid::Uuid::new_v4().to_string(),
        repository: payload.repository.clone(),
        commit_hash: payload.commit_hash.clone(),
        program_id: payload.program_id.clone(),
        lib_name: payload.lib_name.clone(),
        bpf_flag: payload.bpf_flag.unwrap_or(false),
        created_at: Utc::now().naive_utc(),
        base_docker_image: payload.base_image.clone(),
        mount_path: payload.mount_path.clone(),
        cargo_args: payload.cargo_args.clone(),
        status: JobStatus::InProgress.into(),
    };

    // First check if the program is already verified
    let is_exists = db
        .check_is_build_params_exists_already(&payload)
        .await
        .unwrap_or((false, None));

    if is_exists.0 {
        if let Some(res) = is_exists.1 {
            return (
                StatusCode::CONFLICT,
                Json(
                    StatusResponse {
                        is_verified: res.is_verified,
                        message: if res.is_verified {
                            "On chain program verified".to_string()
                        } else {
                            "On chain program not verified".to_string()
                        },
                        on_chain_hash: res.on_chain_hash,
                        executable_hash: res.executable_hash,
                        repo_url: res.repo_url,
                    }
                    .into(),
                ),
            );
        }
        return (
            StatusCode::CONFLICT,
            Json(
                StatusResponse {
                    is_verified: false,
                    message: "We have already processed this request".to_string(),
                    on_chain_hash: "".to_string(),
                    executable_hash: "".to_string(),
                    repo_url: verify_build_data
                        .commit_hash
                        .map_or(verify_build_data.repository.clone(), |hash| {
                            format!("{}/commit/{}", verify_build_data.repository, hash)
                        }),
                }
                .into(),
            ),
        );
    }

    // insert into database
    if let Err(e) = db.insert_build_params(&verify_build_data).await {
        tracing::error!("Error inserting into database: {:?}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                ErrorResponse {
                    status: Status::Error,
                    error: "An unexpected database error occurred.".to_string(),
                }
                .into(),
            ),
        );
    }

    tracing::info!("Inserted into database");

    // run task and wait for it to finish
    match verify_build(payload).await {
        Ok(res) => {
            let _ = db.insert_or_update_verified_build(&res).await;
            let _ = db
                .update_build_status(&verify_build_data.id, JobStatus::Completed.into())
                .await;
            (
                StatusCode::OK,
                Json(
                    StatusResponse {
                        is_verified: res.is_verified,
                        message: if res.is_verified {
                            "On chain program verified".to_string()
                        } else {
                            "On chain program not verified".to_string()
                        },
                        on_chain_hash: res.on_chain_hash,
                        executable_hash: res.executable_hash,
                        repo_url: verify_build_data
                            .commit_hash
                            .map_or(verify_build_data.repository.clone(), |hash| {
                                format!("{}/commit/{}", verify_build_data.repository, hash)
                            }),
                    }
                    .into(),
                ),
            )
        }
        Err(err) => {
            tracing::error!("Error verifying build: {:?}", err);
            (
                StatusCode::OK,
                Json(
                    ErrorResponse {
                        status: Status::Error,
                        error:
                            "We encountered an unexpected error during the verification process."
                                .to_string(),
                    }
                    .into(),
                ),
            )
        }
    }
}
