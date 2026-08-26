locals {
  name          = "piqae-${var.environment}"
  min_instances = var.environment == "production" ? 3 : 0
  max_instances = var.environment == "production" ? 10 : 2
  database_url = var.enable_managed_data_plane ? format(
    "postgresql://piqae:%s@localhost/piqae?host=%s",
    urlencode(random_password.database[0].result),
    urlencode("/cloudsql/${google_sql_database_instance.primary[0].connection_name}")
  ) : var.database_url_secret
}

resource "google_project_service" "run" {
  service            = "run.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "artifact_registry" {
  service            = "artifactregistry.googleapis.com"
  disable_on_destroy = false
}

resource "google_project_service" "secret_manager" {
  service            = "secretmanager.googleapis.com"
  disable_on_destroy = false
}

resource "google_service_account" "server" {
  account_id   = local.name
  display_name = "Piqae ${var.environment} control plane"
}

resource "google_secret_manager_secret" "database_url" {
  secret_id  = "${local.name}-database-url"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "database_url" {
  secret      = google_secret_manager_secret.database_url.id
  secret_data = local.database_url
}

resource "google_secret_manager_secret" "object_access_key" {
  secret_id  = "${local.name}-object-access-key"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "object_access_key" {
  secret      = google_secret_manager_secret.object_access_key.id
  secret_data = var.object_store_access_key_secret
}

resource "google_secret_manager_secret" "object_secret_key" {
  secret_id  = "${local.name}-object-secret-key"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "object_secret_key" {
  secret      = google_secret_manager_secret.object_secret_key.id
  secret_data = var.object_store_secret_key_secret
}

resource "google_secret_manager_secret" "webhook_master_key" {
  secret_id  = "${local.name}-webhook-master-key"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "webhook_master_key" {
  secret      = google_secret_manager_secret.webhook_master_key.id
  secret_data = var.webhook_master_key_secret
}

resource "google_secret_manager_secret" "destination_identity_key" {
  secret_id  = "${local.name}-destination-identity-key"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "destination_identity_key" {
  secret      = google_secret_manager_secret.destination_identity_key.id
  secret_data = var.destination_identity_key_secret
}

resource "google_secret_manager_secret" "document_master_key" {
  secret_id  = "${local.name}-document-master-key"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "document_master_key" {
  secret      = google_secret_manager_secret.document_master_key.id
  secret_data = var.document_master_key_secret
}

resource "google_secret_manager_secret" "document_decryption_keys" {
  secret_id  = "${local.name}-document-decryption-keys"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "document_decryption_keys" {
  secret      = google_secret_manager_secret.document_decryption_keys.id
  secret_data = var.document_decryption_keys_secret
}

resource "google_secret_manager_secret" "stripe_secret_key" {
  secret_id  = "${local.name}-stripe-secret-key"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "stripe_secret_key" {
  secret      = google_secret_manager_secret.stripe_secret_key.id
  secret_data = var.stripe_secret_key_secret
}

resource "google_secret_manager_secret" "stripe_webhook_secret" {
  secret_id  = "${local.name}-stripe-webhook-secret"
  depends_on = [google_project_service.secret_manager]
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "stripe_webhook_secret" {
  secret      = google_secret_manager_secret.stripe_webhook_secret.id
  secret_data = var.stripe_webhook_secret
}

resource "google_secret_manager_secret_iam_member" "runtime_secrets" {
  for_each = toset([
    google_secret_manager_secret.database_url.id,
    google_secret_manager_secret.object_access_key.id,
    google_secret_manager_secret.object_secret_key.id,
    google_secret_manager_secret.webhook_master_key.id,
    google_secret_manager_secret.destination_identity_key.id,
    google_secret_manager_secret.document_master_key.id,
    google_secret_manager_secret.document_decryption_keys.id,
    google_secret_manager_secret.stripe_secret_key.id,
    google_secret_manager_secret.stripe_webhook_secret.id,
  ])
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.server.email}"
}

resource "google_project_iam_member" "cloud_sql_client" {
  count   = var.enable_managed_data_plane ? 1 : 0
  project = var.gcp_project_id
  role    = "roles/cloudsql.client"
  member  = "serviceAccount:${google_service_account.server.email}"
}

resource "google_cloud_run_v2_service" "server" {
  for_each            = toset(["api", "sync", "worker"])
  name                = "${local.name}-${each.key}"
  location            = var.gcp_region
  deletion_protection = var.environment == "production"
  ingress             = var.enable_global_load_balancer ? "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER" : "INGRESS_TRAFFIC_ALL"

  depends_on = [google_project_service.run]

  template {
    service_account = google_service_account.server.email
    timeout         = "60s"
    # V1 object transfers are bounded to 50 MiB but are buffered while their
    # digest is verified. Keep per-instance memory use bounded until the object
    # store interface supports end-to-end streaming.
    max_instance_request_concurrency = 4

    dynamic "volumes" {
      for_each = var.enable_managed_data_plane ? [1] : []
      content {
        name = "cloudsql"
        cloud_sql_instance {
          instances = [google_sql_database_instance.primary[0].connection_name]
        }
      }
    }

    scaling {
      min_instance_count = local.min_instances
      max_instance_count = local.max_instances
    }

    containers {
      image = var.image

      ports {
        name           = "h2c"
        container_port = 8080
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "1Gi"
        }
        cpu_idle = false
      }

      dynamic "volume_mounts" {
        for_each = var.enable_managed_data_plane ? [1] : []
        content {
          name       = "cloudsql"
          mount_path = "/cloudsql"
        }
      }

      env {
        name  = "PIQAE_ENVIRONMENT"
        value = var.environment
      }
      env {
        name  = "PIQAE_DEPLOYMENT"
        value = "cloud"
      }
      env {
        name  = "PIQAE_SERVICE_ROLE"
        value = each.key
      }
      env {
        name  = "PIQAE_RUN_MIGRATIONS_ON_STARTUP"
        value = "false"
      }
      env {
        name  = "PIQAE_IDENTITY_PROVIDER"
        value = "workos"
      }
      env {
        name  = "PIQAE_BILLING_ENABLED"
        value = "true"
      }
      env {
        name  = "STRIPE_METER_EVENT_NAME"
        value = var.stripe_meter_event_name
      }
      env {
        name  = "PIQAE_BIND"
        value = "0.0.0.0:8080"
      }
      env {
        name  = "PIQAE_AUTH_MODE"
        value = var.auth_mode
      }
      env {
        name  = "PIQAE_OIDC_ISSUER"
        value = var.oidc_issuer
      }
      env {
        name  = "PIQAE_OIDC_JWKS_URL"
        value = var.oidc_jwks_url
      }
      env {
        name  = "PIQAE_OIDC_AUDIENCE"
        value = var.oidc_audience
      }
      env {
        name  = "PIQAE_OIDC_BINDING_CLAIM"
        value = var.oidc_binding_claim
      }
      env {
        name  = "PIQAE_OIDC_BINDING_VALUE"
        value = var.oidc_binding_value
      }
      env {
        name  = "PIQAE_OIDC_ORGANIZATION_CLAIM"
        value = var.oidc_organization_claim
      }
      env {
        name  = "PIQAE_OIDC_PERMISSIONS_CLAIM"
        value = var.oidc_permissions_claim
      }
      env {
        name  = "PIQAE_OIDC_ALLOW_UNRESTRICTED"
        value = "false"
      }
      env {
        name  = "PIQAE_OBJECT_STORE"
        value = var.enable_managed_data_plane ? "gcs" : "s3"
      }
      env {
        name  = "PIQAE_GCS_BUCKET"
        value = var.enable_managed_data_plane ? google_storage_bucket.objects[0].name : ""
      }
      env {
        name  = "PIQAE_S3_ENDPOINT"
        value = var.object_store_endpoint
      }
      env {
        name  = "PIQAE_S3_BUCKET"
        value = var.object_store_bucket
      }
      env {
        name  = "PIQAE_S3_REGION"
        value = "auto"
      }
      env {
        name = "PIQAE_DATABASE_URL"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.database_url.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "PIQAE_S3_ACCESS_KEY_ID"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.object_access_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "PIQAE_S3_SECRET_ACCESS_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.object_secret_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "PIQAE_WEBHOOK_MASTER_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.webhook_master_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "PIQAE_DESTINATION_IDENTITY_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.destination_identity_key.secret_id
            version = google_secret_manager_secret_version.destination_identity_key.version
          }
        }
      }
      env {
        name = "PIQAE_DOCUMENT_MASTER_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.document_master_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name  = "PIQAE_DOCUMENT_ACTIVE_KEY_ID"
        value = var.document_active_key_id
      }
      env {
        name  = "PIQAE_DOCUMENT_ARTIFACT_DOWNLOAD_CONCURRENCY"
        value = tostring(var.document_artifact_download_concurrency)
      }
      env {
        name = "PIQAE_DOCUMENT_DECRYPTION_KEYS"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.document_decryption_keys.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "STRIPE_SECRET_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.stripe_secret_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "STRIPE_WEBHOOK_SECRET"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.stripe_webhook_secret.secret_id
            version = "latest"
          }
        }
      }

      startup_probe {
        failure_threshold     = 12
        initial_delay_seconds = 1
        period_seconds        = 2
        timeout_seconds       = 1
        http_get {
          path = "/v1/ready"
          port = 8080
        }
      }

      liveness_probe {
        failure_threshold = 3
        period_seconds    = 10
        timeout_seconds   = 2
        http_get {
          path = "/v1/health"
          port = 8080
        }
      }
    }
  }

  lifecycle {
    precondition {
      condition = !contains(["oidc", "hybrid"], var.auth_mode) || (
        var.oidc_jwks_url != "" &&
        ((var.oidc_audience != "") != (var.oidc_binding_value != ""))
      )
      error_message = "OIDC requires oidc_jwks_url and exactly one of oidc_audience or oidc_binding_value."
    }
  }

  traffic {
    type    = "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST"
    percent = 100
  }
}

resource "google_cloud_run_v2_job" "migration" {
  name                = "${local.name}-migrate"
  location            = var.gcp_region
  deletion_protection = var.environment == "production"

  template {
    template {
      service_account = google_service_account.server.email
      max_retries     = 1
      timeout         = "600s"

      dynamic "volumes" {
        for_each = var.enable_managed_data_plane ? [1] : []
        content {
          name = "cloudsql"
          cloud_sql_instance {
            instances = [google_sql_database_instance.primary[0].connection_name]
          }
        }
      }

      containers {
        image = var.migration_image
        dynamic "volume_mounts" {
          for_each = var.enable_managed_data_plane ? [1] : []
          content {
            name       = "cloudsql"
            mount_path = "/cloudsql"
          }
        }
        env {
          name = "DATABASE_URL"
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.database_url.secret_id
              version = "latest"
            }
          }
        }
      }
    }
  }
}

resource "google_cloud_run_v2_service_iam_member" "public" {
  for_each = var.allow_public_cloud_run_invocation ? toset(["api", "sync"]) : toset([])
  location = google_cloud_run_v2_service.server[each.key].location
  name     = google_cloud_run_v2_service.server[each.key].name
  role     = "roles/run.invoker"
  member   = "allUsers"
}
