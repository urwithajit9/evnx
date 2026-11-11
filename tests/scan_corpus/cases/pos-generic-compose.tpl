services:
  db:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD: {{gen:alnum:24:generic-compose-1}}
