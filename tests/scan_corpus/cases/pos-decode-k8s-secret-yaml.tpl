apiVersion: v1
kind: Secret
metadata:
  name: registry
data:
  token: {{b64:rule:github-token}}
  note: {{b64:cat:rotate this every quarter}}
