services:
  app:
    image: acme/app:1.4.2
    environment:
      DATABASE_URL: {{rule:connection-string-credentials}}
