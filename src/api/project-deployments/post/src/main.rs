use aws_sdk_codebuild::model::{EnvironmentVariable, EnvironmentVariableType};
use error_stack::{Report, ResultExt};
use lambda_runtime::{service_fn, LambdaEvent};
use serde_json::{json, Value};

use buildor::{
    handlers::{project_deployments::ProjectDeploymentsHandler, projects::ProjectsHandler},
    models::{
        common::ExecutionError,
        handlers::{HandlerCreate, HandlerGet},
        project_deployment::{
            ProjectDeploymentCreatePayload, ProjectDeploymentCreatePayloadRequest,
            ProjectDeploymentError,
        },
        request::RequestError,
        response::Response,
    },
    utils::{get_build_info, load_env_var, parse_request_body_payload, Clients},
};

#[tokio::main]
async fn main() -> Result<(), Value> {
    env_logger::init();

    println!("Creating service fn for handler");
    let func = service_fn(handler);
    println!("Executing handler from runtime");
    let result = lambda_runtime::run(func).await;
    println!("Evaluating handler result");
    match result {
        Ok(res) => {
            println!("Success");
            Ok(res)
        }
        Err(err) => {
            println!("Handler exception: {}", err);
            Err(json!(RequestError::internal()))
        }
    }
}

async fn handler(event: LambdaEvent<Value>) -> Result<Value, Report<ExecutionError>> {
    println!("Start handler execution");

    println!("Load env vars");
    #[allow(non_snake_case)]
    let TABLE_NAME = load_env_var("TABLE_NAME", None).change_context(ExecutionError)?;
    #[allow(non_snake_case)]
    let TABLE_NAME_PROJECTS =
        load_env_var("TABLE_NAME_PROJECTS", None).change_context(ExecutionError)?;
    #[allow(non_snake_case)]
    let TABLE_REGION = load_env_var("TABLE_REGION", None).change_context(ExecutionError)?;
    println!("TABLE_NAME: {}", TABLE_NAME);
    println!("TABLE_REGION: {}", TABLE_REGION);

    println!("Parse event and context objects");
    let (event, context) = event.into_parts();
    println!("event: {:?}", event);
    println!("context: {:?}", context);

    // Body Payload
    println!("Parse body payload");
    let body =
        match parse_request_body_payload::<ProjectDeploymentCreatePayloadRequest>(&event["body"]) {
            Ok(value) => value,
            Err(err) => return Ok(json!(err)),
        };
    println!("Body: {:?}", body);

    let ph = ProjectsHandler::new(Clients::dynamodb().await, TABLE_NAME_PROJECTS);

    // Fetch Project
    let project = match ph.get(body.project_uuid).await {
        Ok(value) => match value {
            Some(project) => project,
            None => return Ok(json!({"error": "Item not found"})),
        },
        Err(error) => {
            println!("Failed to get project: {}", error);
            return Ok(json!({"error": "Item not found"}));
        }
    };
    println!("Project: {:?}", project);

    // CodeBuild Vars
    let codebuild_project_name = "App-Dynamically-Deploy-SPAs";
    let project_name = "buildspace-solana-pay";
    let repo_url = "https://github.com/ccddan/buildspace-solana-pay.git";
    let timestamp = "timestamp";
    let output_folder = "out";

    let move_output_folder_command = format!("mv {output_folder} ../dist");

    let pre_build_commands = Vec::from([
        "echo Install project dependencies",
        "cd $PROJECT_NAME",
        "npm install",
    ]);
    let build_commands = Vec::from([
        "echo Build project",
        "npm run release",
        "echo Move build output to artifacts location",
        &move_output_folder_command,
        "cd ..",
        "ls -las dist",
    ]);

    let pre_build_commands_str = pre_build_commands
        .iter()
        .map(|s| format!("\"{}\"", s.to_string()))
        .collect::<Vec<String>>()
        .join(",");
    let build_commands_str = build_commands
        .iter()
        .map(|s| format!("\"{}\"", s.to_string()))
        .collect::<Vec<String>>()
        .join(",");

    let artifacts_output_name = format!("{project_name}-dist-{timestamp}.zip");
    let build_spec = format!(
        r###"
        {{
          "version": "0.2",
          "env": {{
            "variables": {{
              "MY_ENV_VAR": "value"
            }}
          }},
          "phases": {{
            "install": {{
              "commands": [
                "echo Download project",
                "node -v",
                "git clone $REPO_URL $PROJECT_NAME"
              ]
            }},
            "pre_build": {{
              "commands": [{pre_build_commands_str}]
            }},
            "build": {{
              "commands": [{build_commands_str}]
            }},
            "post_build": {{
              "commands": ["echo Build has completed and artifacts were moved"]
            }}
          }},
          "artifacts": {{
            "discard-paths": "no",
            "files": ["dist/**/*"],
            "name": "{artifacts_output_name}"
          }}
        }}
        "###
    );

    let build = Clients::codebuild().await;
    let pdh = ProjectDeploymentsHandler::new(Clients::dynamodb().await, TABLE_NAME);
    let tx = build
        .start_build()
        .project_name(codebuild_project_name.to_string())
        .environment_variables_override(
            EnvironmentVariable::builder()
                .set_name(Some("PROJECT_NAME".to_string()))
                .set_value(Some(project_name.to_string()))
                .set_type(Some(EnvironmentVariableType::Plaintext))
                .build(),
        )
        .environment_variables_override(
            EnvironmentVariable::builder()
                .set_name(Some("REPO_URL".to_string()))
                .set_value(Some(repo_url.to_string()))
                .set_type(Some(EnvironmentVariableType::Plaintext))
                .build(),
        )
        .buildspec_override(build_spec);

    match tx.send().await {
        Ok(result) => {
            println!("Result: {:?}", result);
            println!("Parse build info");
            if let Some(build_info) = get_build_info(&result) {
                println!("Create project deployment record");
                return match pdh
                    .create(ProjectDeploymentCreatePayload {
                        project,
                        build: build_info,
                    })
                    .await
                {
                    Ok(value) => Ok(Response::new(json!(value), 201)),
                    Err(error) => {
                        println!(
                            "Failed to create project deployment record: {}",
                            error.change_context(ExecutionError)
                        );
                        Ok(Response::new(
                            json!(ProjectDeploymentError::creation_failed()),
                            400,
                        ))
                    }
                };
            } else {
                return Ok(Response::new(
                    json!({"error":"Failed to parse build info, but deployment has been triggered (probably)" }),
                    400,
                ));
            }
        }
        Err(err) => {
            println!("Error: {}", err);
            Ok(Response::new(json!({ "error": format!("{}", err) }), 200))
        }
    }
}
