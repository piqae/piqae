locals {
  name          = "spool-${var.environment}"
  min_instances = var.environment == "production" ? 3 : 0
  max_instances = var.environment == "production" ? 10 : 2
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
  display_name = "Spool ${var.environment} control plane"
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
  secret_data = var.database_url_secret
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

resource "google_secret_manager_secret_iam_member" "runtime_secrets" {
  for_each = toset([
    google_secret_manager_secret.database_url.id,
    google_secret_manager_secret.object_access_key.id,
    google_secret_manager_secret.object_secret_key.id,
    google_secret_manager_secret.webhook_master_key.id,
  ])
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.server.email}"
}

resource "google_cloud_run_v2_service" "server" {
  name                = local.name
  location            = var.gcp_region
  deletion_protection = var.environment == "production"
  ingress             = "INGRESS_TRAFFIC_ALL"

  depends_on = [google_project_service.run]

  scaling {
    min_instance_count = local.min_instances
    max_instance_count = local.max_instances
  }

  template {
    service_account = google_service_account.server.email
    timeout         = "60s"
    # V1 object transfers are bounded to 50 MiB but are buffered while their
    # digest is verified. Keep per-instance memory use bounded until the object
    # store interface supports end-to-end streaming.
    max_instance_request_concurrency = 4

    lifecycle {
      precondition {
        condition = !contains(["oidc", "hybrid"], var.auth_mode) || (
          var.oidc_jwks_url != "" &&
          ((var.oidc_audience != "") != (var.oidc_binding_value != ""))
        )
        error_message = "OIDC requires oidc_jwks_url and exactly one of oidc_audience or oidc_binding_value."
      }
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

      env {
        name  = "SPOOL_ENVIRONMENT"
        value = var.environment
      }
      env {
        name  = "SPOOL_BIND"
        value = "0.0.0.0:8080"
      }
      env {
        name  = "SPOOL_AUTH_MODE"
        value = var.auth_mode
      }
      env {
        name  = "SPOOL_OIDC_ISSUER"
        value = var.oidc_issuer
      }
      env {
        name  = "SPOOL_OIDC_JWKS_URL"
        value = var.oidc_jwks_url
      }
      env {
        name  = "SPOOL_OIDC_AUDIENCE"
        value = var.oidc_audience
      }
      env {
        name  = "SPOOL_OIDC_BINDING_CLAIM"
        value = var.oidc_binding_claim
      }
      env {
        name  = "SPOOL_OIDC_BINDING_VALUE"
        value = var.oidc_binding_value
      }
      env {
        name  = "SPOOL_OIDC_ORGANIZATION_CLAIM"
        value = var.oidc_organization_claim
      }
      env {
        name  = "SPOOL_OIDC_PERMISSIONS_CLAIM"
        value = var.oidc_permissions_claim
      }
      env {
        name  = "SPOOL_OIDC_ALLOW_UNRESTRICTED"
        value = "false"
      }
      env {
        name  = "SPOOL_OBJECT_STORE"
        value = "s3"
      }
      env {
        name  = "SPOOL_S3_ENDPOINT"
        value = var.object_store_endpoint
      }
      env {
        name  = "SPOOL_S3_BUCKET"
        value = var.object_store_bucket
      }
      env {
        name  = "SPOOL_S3_REGION"
        value = "auto"
      }
      env {
        name = "SPOOL_DATABASE_URL"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.database_url.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "SPOOL_S3_ACCESS_KEY_ID"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.object_access_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "SPOOL_S3_SECRET_ACCESS_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.object_secret_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "SPOOL_WEBHOOK_MASTER_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.webhook_master_key.secret_id
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
          path = "/v1/health"
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

  traffic {
    type    = "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST"
    percent = 100
  }
}

resource "google_cloud_run_v2_service_iam_member" "public" {
  location = google_cloud_run_v2_service.server.location
  name     = google_cloud_run_v2_service.server.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}
