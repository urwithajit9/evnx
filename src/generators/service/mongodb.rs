//! Generator for MongoDB database service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for MongoDB database service.
pub struct MongoDbGenerator;

impl ServiceGenerator for MongoDbGenerator {
    fn id(&self) -> &'static str {
        "mongodb"
    }

    fn display_name(&self) -> &'static str {
        "MongoDB"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![EnvVar::new("MONGODB_URI", "mongodb://localhost:27017/mydb")
            .with_description("MongoDB connection URI")
            .required()
            .with_category("Database")]
    }
}
