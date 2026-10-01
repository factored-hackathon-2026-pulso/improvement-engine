# Service boundary

Implement detection and autonomous improvement, consuming Agent Core primitives and external model configuration. Do not implement another Agent Core or an LLM gateway. Windows first. The engine owns its development/integration dependencies (Compose, LocalStack, fixtures and ephemeral CI services); sibling `infra` owns Terraform, AWS deployment and operational infrastructure. Documentation changes alongside each slice, distinguishing planned contracts and implemented behavior.
