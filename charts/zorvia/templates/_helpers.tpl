{{/*
Expand the name of the chart.
*/}}
{{- define "zorvia.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "zorvia.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "zorvia.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "zorvia.labels" -}}
helm.sh/chart: {{ include "zorvia.chart" . }}
{{ include "zorvia.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: zorvia
{{- end }}

{{- define "zorvia.selectorLabels" -}}
app.kubernetes.io/name: {{ include "zorvia.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: api
{{- end }}

{{- define "zorvia.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "zorvia.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "zorvia.authSecretName" -}}
{{- if .Values.auth.existingSecret }}
{{- .Values.auth.existingSecret }}
{{- else if .Values.auth.secretName }}
{{- .Values.auth.secretName }}
{{- else }}
{{- printf "%s-auth" (include "zorvia.fullname" .) }}
{{- end }}
{{- end }}

{{- define "zorvia.image" -}}
{{- if .Values.image.digest }}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest }}
{{- else }}
{{- printf "%s:%s" .Values.image.repository (.Values.image.tag | toString) }}
{{- end }}
{{- end }}

{{/*
True when the control plane runs more than one replica.
*/}}
{{- define "zorvia.multiReplica" -}}
{{- if gt (int .Values.replicaCount) 1 -}}true{{- end -}}
{{- end }}

{{/*
Single replica: state is SQLite files under /data on one volume. More than one
replica needs the shared PostgreSQL store for users, operations, audit and the
JSON documents (docs/POSTGRES.md), leader election for the background loops, and
an explicit acknowledgement of what is still per replica.
*/}}
{{- define "zorvia.validateHA" -}}
{{- if gt (int .Values.replicaCount) 1 -}}
{{- if not (or .Values.database.url .Values.database.existingSecret) -}}
{{- fail "replicaCount > 1 needs the PostgreSQL store: set database.existingSecret (or database.url). Without it the state is SQLite files that two pods must not share. See docs/POSTGRES.md." -}}
{{- end -}}
{{- if not .Values.api.leaderElection -}}
{{- fail "replicaCount > 1 needs api.leaderElection: true, otherwise every replica runs the schedulers and the operations reconciler." -}}
{{- end -}}
{{- if not .Values.ha.acceptLocalState -}}
{{- fail "replicaCount > 1: commercial-offerings records (ZORVIA_COMMERCIAL_DB) and the JSONL audit sidecar stay per replica and are lost with the pod. Set ha.acceptLocalState: true to confirm you do not use them. See docs/POSTGRES.md." -}}
{{- end -}}
{{- end -}}
{{- end }}
