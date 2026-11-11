FROM node@sha256:{{hash:64:neg-from-digest}}
ARG VERSION=1.2.3
ARG BUILD_DATE
ENV NODE_ENV=production
ENV PATH=/app/node_modules/.bin:$PATH
LABEL org.opencontainers.image.revision={{hash:40:neg-label-rev}}
COPY --chown=node:node . /app
CMD ["node", "server.js"]
