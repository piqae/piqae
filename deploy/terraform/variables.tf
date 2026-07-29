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

variable "public_api_origin" {
  type = string
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
