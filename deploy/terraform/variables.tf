variable "gcp_project_id" {
  type        = string
  description = "Dedicated GCP project for the Spool environment."
}

variable "gcp_region" {
  type        = string
  description = "Cloud Run and Artifact Registry region."
  default     = "australia-southeast1"
}

variable "environment" {
  type        = string
  description = "Deployment environment."
  validation {
    condition     = contains(["staging", "production"], var.environment)
    error_message = "environment must be staging or production"
  }
}

variable "image" {
  type        = string
  description = "Immutable spool-server OCI image digest."
}

variable "database_url_secret" {
  type        = string
  sensitive   = true
  description = "Neon pooled PostgreSQL connection URL."
}

variable "object_store_endpoint" {
  type        = string
  description = "Cloudflare R2 S3-compatible endpoint."
}

variable "object_store_bucket" {
  type = string
}

variable "object_store_access_key_secret" {
  type      = string
  sensitive = true
}

variable "object_store_secret_key_secret" {
  type      = string
  sensitive = true
}

variable "webhook_master_key_secret" {
  type        = string
  sensitive   = true
  description = "Base64-encoded 32-byte key used to encrypt webhook signing secrets."
}

variable "auth_mode" {
  type        = string
  description = "Control-plane authentication mode. Hosted deployments should use oidc."
  default     = "oidc"
  validation {
    condition     = contains(["api_key", "bootstrap", "hybrid", "oidc"], var.auth_mode)
    error_message = "auth_mode must be api_key, bootstrap, hybrid, or oidc"
  }
}

variable "oidc_issuer" {
  type        = string
  description = "Exact trusted OIDC issuer, including its trailing slash when the issuer publishes one."
  default     = "https://api.workos.com/"
}

variable "oidc_jwks_url" {
  type        = string
  description = "HTTPS JWKS endpoint for the trusted identity application."
  default     = ""
}

variable "oidc_audience" {
  type        = string
  description = "Expected standard aud claim. Leave empty when binding through oidc_binding_value."
  default     = ""
}

variable "oidc_binding_claim" {
  type        = string
  description = "Verified application-binding claim used when the provider does not issue aud."
  default     = "client_id"
}

variable "oidc_binding_value" {
  type        = string
  description = "Expected application identifier. WorkOS deployments should set this to WORKOS_CLIENT_ID."
  default     = ""
}

variable "oidc_organization_claim" {
  type        = string
  description = "Verified claim that selects the tenant organization."
  default     = "org_id"
}

variable "oidc_permissions_claim" {
  type        = string
  description = "Verified array claim mapped to Spool API scopes."
  default     = "permissions"
}

variable "enable_multi_region" {
  type        = bool
  description = "Create a warm Cloud Run service and, when managed data is enabled, a PostgreSQL DR replica in Melbourne."
  default     = false
}

variable "secondary_region" {
  type        = string
  description = "Warm-standby region. australia-southeast2 is Melbourne."
  default     = "australia-southeast2"
}

variable "secondary_min_instances" {
  type        = number
  description = "Warm Cloud Run capacity held in the secondary region."
  default     = 1
  validation {
    condition     = var.secondary_min_instances >= 0
    error_message = "secondary_min_instances must be non-negative"
  }
}

variable "enable_global_load_balancer" {
  type        = bool
  description = "Create a global external HTTPS load balancer across both serverless NEGs."
  default     = false
}

variable "load_balancer_domains" {
  type        = list(string)
  description = "DNS names on the Google-managed load-balancer certificate."
  default     = []
  validation {
    condition     = !var.enable_global_load_balancer || length(var.load_balancer_domains) > 0
    error_message = "load_balancer_domains must not be empty when the load balancer is enabled"
  }
}

variable "allow_public_cloud_run_invocation" {
  type        = bool
  description = "Grant allUsers Cloud Run invocation. Spool application authentication still applies."
  default     = false
  validation {
    condition     = !var.enable_global_load_balancer || var.allow_public_cloud_run_invocation
    error_message = "allow_public_cloud_run_invocation must be true for an external serverless NEG; Spool application auth remains enforced"
  }
}

variable "enable_managed_data_plane" {
  type        = bool
  description = "Create optional Cloud SQL HA/DR and a custom dual-region GCS bucket. Existing external data services remain the default."
  default     = false
}

variable "cloud_sql_tier" {
  type        = string
  description = "Cloud SQL machine tier for primary and DR replica."
  default     = "db-custom-2-7680"
}

variable "cloud_sql_disk_size_gb" {
  type        = number
  description = "Initial primary Cloud SQL SSD size; autoresize remains enabled."
  default     = 100
}

variable "managed_object_bucket_name" {
  type        = string
  description = "Globally unique GCS bucket name, required when enable_managed_data_plane=true."
  default     = ""
  validation {
    condition     = !var.enable_managed_data_plane || length(var.managed_object_bucket_name) >= 3
    error_message = "managed_object_bucket_name is required when managed data is enabled"
  }
}
