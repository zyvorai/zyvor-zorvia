variable "zorvia_url" {
  type        = string
  description = "Zorvia API base URL (https://HOST:30152)"
}

variable "zorvia_token" {
  type        = string
  sensitive   = true
  description = "JWT or API key"
}

variable "name" {
  type        = string
  description = "VM name (RFC 1123)"
}

variable "image" {
  type        = string
  default     = "quay.io/containerdisks/ubuntu:24.04"
}

variable "cpus" {
  type    = number
  default = 2
}

variable "memory" {
  type        = number
  default     = 2048
  description = "Memory in MiB"
}

variable "start" {
  type    = bool
  default = true
}

variable "expose_ssh" {
  type    = bool
  default = false
}

variable "insecure" {
  type        = bool
  default     = true
  description = "Pass -k to curl (lab TLS)"
}
