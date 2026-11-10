use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub access_ttl_minutes: i64,
    pub refresh_ttl_days: i64,
    pub cors_origins: Vec<String>,
    pub run_jobs: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let _ = dotenvy::dotenv();
        let jwt_secret = env::var("JWT_SECRET").map_err(|_| "JWT_SECRET must be set".to_string())?;
        if jwt_secret.len() < 32 {
            return Err("JWT_SECRET must be at least 32 characters".into());
        }
        Ok(Config {
            database_url: env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set".to_string())?,
            bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            jwt_secret,
            access_ttl_minutes: env::var("ACCESS_TTL_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(60),
            refresh_ttl_days: env::var("REFRESH_TTL_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(14),
            cors_origins: env::var("CORS_ORIGINS").unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            run_jobs: env::var("RUN_JOBS").map(|v| v != "0" && v != "false").unwrap_or(true),
        })
    }
}
