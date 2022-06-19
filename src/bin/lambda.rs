use qk_run;
use rocket;
use lambda_web::{launch_rocket_on_lambda, LambdaError};

#[rocket::main]
async fn main() -> Result<(), LambdaError> {
    launch_rocket_on_lambda(qk_run::rocket()).await?;
    Ok(())
}
