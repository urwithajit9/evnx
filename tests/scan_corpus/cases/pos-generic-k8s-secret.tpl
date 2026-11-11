apiVersion: v1
kind: Secret
metadata:
  name: app-secrets
stringData:
  DB_PASSWORD: {{gen:alnum:24:generic-k8s-1}}
  API_TOKEN: {{gen:alnum:40:generic-k8s-2}}
