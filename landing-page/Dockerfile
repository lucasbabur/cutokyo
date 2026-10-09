FROM oven/bun:1.3.12-slim@sha256:d3c7094c144dd3975d183a4dbc4ec0a764223995bff73290d983edb47043a75f AS dependencies

WORKDIR /app
COPY package.json bun.lock ./
RUN bun install --frozen-lockfile --ignore-scripts


FROM dependencies AS builder

ARG NEXT_PUBLIC_APP_URL=http://localhost:3000
ARG NEXT_PUBLIC_FASTAPI_BASE_URL=http://localhost:8000
ARG NEXT_PUBLIC_FASTAPI_OPENAPI_URL=http://localhost:8000/openapi.json
ARG NEXT_PUBLIC_WORKOS_REDIRECT_URI=http://localhost:3000/auth/callback
ARG NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING=off

ENV NEXT_OUTPUT=standalone \
    NEXT_PUBLIC_APP_ENV=production \
    NEXT_PUBLIC_APP_URL=${NEXT_PUBLIC_APP_URL} \
    NEXT_PUBLIC_FASTAPI_BASE_URL=${NEXT_PUBLIC_FASTAPI_BASE_URL} \
    NEXT_PUBLIC_FASTAPI_OPENAPI_URL=${NEXT_PUBLIC_FASTAPI_OPENAPI_URL} \
    NEXT_PUBLIC_WORKOS_REDIRECT_URI=${NEXT_PUBLIC_WORKOS_REDIRECT_URI} \
    NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING=${NEXT_PUBLIC_CUTOKYO_TOKEN_SAVING} \
    NEXT_TELEMETRY_DISABLED=1

COPY . .
RUN bun run build


FROM node:24.4.1-slim@sha256:36ae19f59c91f3303c7a648f07493fe14c4bd91320ac8d898416327bacf1bbfa AS runtime

ENV HOSTNAME=0.0.0.0 \
    NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000

WORKDIR /app
RUN chown node:node /app

COPY --from=builder --chown=node:node /app/.next/standalone ./
COPY --from=builder --chown=node:node /app/.next/static ./.next/static
COPY --from=builder --chown=node:node /app/public ./public

USER node

EXPOSE 3000

CMD ["node", "server.js"]
