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

resource "google_service_account" "server" {
  account_id   = local.name
  display_name = "Spool ${var.environment} control plane"
}

resource "google_secret_manager_secret" "database_url" {
  secret_id = "${local.name}-database-url"
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "database_url" {
  secret      = google_secret_manager_secret.database_url.id
  secret_data = var.database_url_secret
}

resource "google_secret_manager_secret" "object_access_key" {
  secret_id = "${local.name}-object-access-key"
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "object_access_key" {
  secret      = google_secret_manager_secret.object_access_key.id
  secret_data = var.object_store_access_key_secret
}

resource "google_secret_manager_secret" "object_secret_key" {
  secret_id = "${local.name}-object-secret-key"
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "object_secret_key" {
  secret      = google_secret_manager_secret.object_secret_key.id
  secret_data = var.object_store_secret_key_secret
}

resource "google_secret_manager_secret_iam_member" "runtime_secrets" {
  for_each = toset([
    google_secret_manager_secret.database_url.id,
    google_secret_manager_secret.object_access_key.id,
    google_secret_manager_secret.object_secret_key.id,
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
    max_instance_request_concurrency = 200

    containers {
      image = var.image

      ports {
        name           = "h2c"
        container_port = 8080
      }

      resources {
        limits = {
          cpu    = "1"
          memory = "512Mi"
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
        name  = "SPOOL_PUBLIC_API_ORIGIN"
        value = var.public_api_origin
      }
      env {
        name  = "SPOOL_OBJECT_STORE_ENDPOINT"
        value = var.object_store_endpoint
      }
      env {
        name  = "SPOOL_OBJECT_STORE_BUCKET"
        value = var.object_store_bucket
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
        name = "SPOOL_OBJECT_STORE_ACCESS_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.object_access_key.secret_id
            version = "latest"
          }
        }
      }
      env {
        name = "SPOOL_OBJECT_STORE_SECRET_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.object_secret_key.secret_id
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
