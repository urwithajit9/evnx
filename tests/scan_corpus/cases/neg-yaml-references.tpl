services:
  app:
    environment:
      DB_PASSWORD: ${DB_PASSWORD}
      API_TOKEN: "{{ .Values.apiToken }}"
      SESSION_SECRET:
    secrets:
      - db_password
env:
  - name: DB_PASSWORD
    valueFrom:
      secretKeyRef:
        name: app-secrets
        key: password
