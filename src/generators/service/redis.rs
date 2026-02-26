//! Generator for Redis cache service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for Redis cache service.
pub struct RedisGenerator;

impl ServiceGenerator for RedisGenerator {
    fn id(&self) -> &'static str {
        "redis"
    }

    fn display_name(&self) -> &'static str {
        "Redis"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![EnvVar::new("REDIS_URL", "redis://localhost:6379/0")
            .with_description("Redis connection URL")
            .with_category("Cache")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_generator() {
        assert_eq!(RedisGenerator.id(), "redis");
        assert_eq!(RedisGenerator.env_vars().len(), 1);
    }
}
