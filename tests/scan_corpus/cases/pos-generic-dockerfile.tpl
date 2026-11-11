FROM node:20-slim
ARG NPM_TOKEN
ENV DB_PASSWORD={{gen:alnum:20:generic-docker-1}}
ENV AWS_ACCESS_KEY_ID={{rule:aws-access-key}}
CMD ["node", "server.js"]
