{{- define "spool.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- define "spool.fullname" -}}
{{- if .Values.fullnameOverride }}{{ .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}{{- else }}{{ printf "%s-%s" .Release.Name (include "spool.name" .) | trunc 63 | trimSuffix "-" }}{{- end }}
{{- end }}
{{- define "spool.labels" -}}
app.kubernetes.io/name: {{ include "spool.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | quote }}
{{- end }}
{{- define "spool.selectorLabels" -}}
app.kubernetes.io/name: {{ include "spool.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
{{- define "spool.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}{{ default (include "spool.fullname" .) .Values.serviceAccount.name }}{{ else }}{{ required "serviceAccount.name is required when create=false" .Values.serviceAccount.name }}{{ end }}
{{- end }}
{{- define "spool.image" -}}
{{- $image := .image -}}
{{- if $image.digest }}{{ printf "%s@%s" $image.repository $image.digest }}{{- else }}{{ printf "%s:%s" $image.repository (required "an image tag or digest is required" $image.tag) }}{{- end }}
{{- end }}
{{- define "spool.secretName" -}}
{{- if .Values.externalSecret.enabled }}{{ include "spool.fullname" . }}-runtime{{- else }}{{ required "secrets.existingSecret is required" .Values.secrets.existingSecret }}{{- end }}
{{- end }}
