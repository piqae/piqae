output "server_uri" {
  value = google_cloud_run_v2_service.server.uri
}

output "service_account" {
  value = google_service_account.server.email
}
