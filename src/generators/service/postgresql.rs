//! Generator for PostgreSQL database service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for PostgreSQL database service.
pub struct PostgresqlGenerator;

impl ServiceGenerator for PostgresqlGenerator {
    fn id(&self) -> &'static str {
        "postgresql"
    }

    fn display_name(&self) -> &'static str {
        "PostgreSQL"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![EnvVar::new(
            "DATABASE_URL",
            "postgresql://user:password@localhost:5432/dbname",
        )
        .with_description("PostgreSQL connection string")
        .required()
        .with_category("Database")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postgresql_generator() {
        assert_eq!(PostgresqlGenerator.id(), "postgresql");
        let vars = PostgresqlGenerator.env_vars();
        assert_eq!(vars.len(), 1);
        assert!(vars[0].required);
    }
}
