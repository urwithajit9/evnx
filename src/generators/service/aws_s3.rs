//! Generator for AWS S3 service.

use crate::generators::{EnvVar, ServiceGenerator};

/// Generator for AWS S3 service.
pub struct AwsS3Generator;

impl ServiceGenerator for AwsS3Generator {
    fn id(&self) -> &'static str {
        "aws_s3"
    }

    fn display_name(&self) -> &'static str {
        "AWS S3"
    }

    fn env_vars(&self) -> Vec<EnvVar> {
        vec![
            EnvVar::new("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE")
                .with_description("AWS access key ID")
                .required()
                .with_category("AWS"),
            EnvVar::new(
                "AWS_SECRET_ACCESS_KEY",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            )
            .with_description("AWS secret access key")
            .required()
            .with_category("AWS"),
            EnvVar::new("AWS_STORAGE_BUCKET_NAME", "your-bucket-name")
                .with_description("S3 bucket name")
                .with_category("AWS"),
            EnvVar::new("AWS_REGION", "us-east-1")
                .with_description("AWS region")
                .with_category("AWS"),
        ]
    }
}
