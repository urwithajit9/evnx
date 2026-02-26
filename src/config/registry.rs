//! Generator registry for managing stack and service generators.
//!
//! This module provides a global singleton registry that maps
//! machine-readable IDs to generator implementations.

use crate::generators::{ServiceGenerator, StackGenerator};
// use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Global registry for available generators.
pub struct GeneratorRegistry {
    stacks: HashMap<&'static str, Box<dyn StackGenerator>>,
    services: HashMap<&'static str, Box<dyn ServiceGenerator>>,
}

impl GeneratorRegistry {
    /// Initialize registry with built-in generators.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            stacks: HashMap::new(),
            services: HashMap::new(),
        };

        // Register built-in stacks
        registry.register_stack(Box::new(crate::generators::stack::PythonGenerator));
        registry.register_stack(Box::new(crate::generators::stack::NodeJsGenerator));
        registry.register_stack(Box::new(crate::generators::stack::RustGenerator));
        registry.register_stack(Box::new(crate::generators::stack::GoGenerator));
        registry.register_stack(Box::new(crate::generators::stack::PhpGenerator));
        registry.register_stack(Box::new(crate::generators::stack::RubyGenerator));
        registry.register_stack(Box::new(crate::generators::stack::OtherGenerator));

        // Register built-in services
        registry.register_service(Box::new(crate::generators::service::PostgresqlGenerator));
        registry.register_service(Box::new(crate::generators::service::RedisGenerator));
        registry.register_service(Box::new(crate::generators::service::MongoDbGenerator));
        registry.register_service(Box::new(crate::generators::service::AwsS3Generator));
        registry.register_service(Box::new(crate::generators::service::StripeGenerator));
        registry.register_service(Box::new(crate::generators::service::TwilioGenerator));
        registry.register_service(Box::new(crate::generators::service::SendGridGenerator));
        registry.register_service(Box::new(crate::generators::service::SentryGenerator));
        registry.register_service(Box::new(crate::generators::service::OpenAiGenerator));

        registry
    }

    /// Register a new stack generator.
    pub fn register_stack(&mut self, generator: Box<dyn StackGenerator>) {
        let id = generator.id();
        self.stacks.insert(id, generator);
    }

    /// Register a new service generator.
    pub fn register_service(&mut self, generator: Box<dyn ServiceGenerator>) {
        let id = generator.id();
        self.services.insert(id, generator);
    }

    /// Get a stack generator by ID.
    #[must_use]
    pub fn get_stack(&self, id: &str) -> Option<&dyn StackGenerator> {
        self.stacks.get(id).map(|g| g.as_ref())
    }

    /// Get a service generator by ID.
    #[must_use]
    pub fn get_service(&self, id: &str) -> Option<&dyn ServiceGenerator> {
        self.services.get(id).map(|g| g.as_ref())
    }

    /// List all available stack IDs.
    #[must_use]
    pub fn list_stacks(&self) -> Vec<&'static str> {
        let mut ids: Vec<_> = self.stacks.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// List all available service IDs.
    #[must_use]
    pub fn list_services(&self) -> Vec<&'static str> {
        let mut ids: Vec<_> = self.services.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}

impl Default for GeneratorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton accessor.
#[must_use]
pub fn registry() -> &'static GeneratorRegistry {
    static REGISTRY: OnceLock<GeneratorRegistry> = OnceLock::new();
    REGISTRY.get_or_init(GeneratorRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_lists_stacks() {
        let reg = registry();
        let stacks = reg.list_stacks();
        assert!(stacks.contains(&"python"));
        assert!(stacks.contains(&"rust"));
    }

    #[test]
    fn test_registry_gets_stack() {
        let reg = registry();
        let stack = reg.get_stack("python").unwrap();
        assert_eq!(stack.id(), "python");
        assert_eq!(stack.display_name(), "Python (Django/FastAPI)");
    }

    #[test]
    fn test_registry_gets_service() {
        let reg = registry();
        let service = reg.get_service("postgresql").unwrap();
        assert_eq!(service.id(), "postgresql");
    }

    #[test]
    fn test_registry_unknown_stack() {
        let reg = registry();
        assert!(reg.get_stack("unknown").is_none());
    }
}
