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
