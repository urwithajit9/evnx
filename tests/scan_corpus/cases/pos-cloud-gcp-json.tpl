{
  "type": "service_account",
  "project_id": "demo-project",
  "private_key": "{{rule:private-key#1}}\n{{gen:b64:64:gcp-pem-1}}\n{{gen:b64:64:gcp-pem-2}}\n-----END PRIVATE KEY-----\n",
  "client_email": "deploy@demo-project.iam.gserviceaccount.com"
}
