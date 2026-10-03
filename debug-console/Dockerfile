# Debug console image: static SPA + same-origin proxy to the control API (plan 17.3.7).
# Pin both base images by digest when the image is built for the LocalServiceManifest (digest is resolved at build time).
ARG NODE_IMAGE=node:22.16-alpine
ARG NGINX_IMAGE=nginxinc/nginx-unprivileged:1.27-alpine

FROM ${NODE_IMAGE} AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json vite.config.ts index.html ./
COPY public ./public
COPY src ./src
RUN npx tsc --noEmit && npx vite build

FROM ${NGINX_IMAGE}
# nginx-unprivileged runs as uid 101 (non-root); /etc/nginx/templates/*.template is rendered by envsubst at start.
ENV PULSO_CONTROL_API_URL=http://pulso-api:8080 \
    NGINX_ENVSUBST_FILTER=PULSO_|NGINX_LOCAL_RESOLVERS \
    NGINX_ENTRYPOINT_LOCAL_RESOLVERS=1
COPY nginx/default.conf.template /etc/nginx/templates/default.conf.template
COPY nginx/security-headers.inc /etc/nginx/conf.d/security-headers.inc
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s --retries=5 CMD wget -qO- http://127.0.0.1:3000/healthz || exit 1
