{{- define "piqae.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- define "piqae.fullname" -}}
{{- if .Values.fullnameOverride }}{{ .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}{{- else }}{{ printf "%s-%s" .Release.Name (include "piqae.name" .) | trunc 63 | trimSuffix "-" }}{{- end }}
{{- end }}
{{- define "piqae.labels" -}}
app.kubernetes.io/name: {{ include "piqae.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | quote }}
{{- end }}
{{- define "piqae.selectorLabels" -}}
app.kubernetes.io/name: {{ include "piqae.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
{{- define "piqae.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}{{ default (include "piqae.fullname" .) .Values.serviceAccount.name }}{{ else }}{{ required "serviceAccount.name is required when create=false" .Values.serviceAccount.name }}{{ end }}
{{- end }}
{{- define "piqae.image" -}}
{{- $image := .image -}}
{{- if $image.digest }}{{ printf "%s@%s" $image.repository $image.digest }}{{- else }}{{ printf "%s:%s" $image.repository (required "an image tag or digest is required" $image.tag) }}{{- end }}
{{- end }}
{{- define "piqae.secretName" -}}
{{- if .Values.externalSecret.enabled }}{{ include "piqae.fullname" . }}-runtime{{- else }}{{ required "secrets.existingSecret is required" .Values.secrets.existingSecret }}{{- end }}
{{- end }}
