export type ProcessIconName = "install" | "auth" | "savings";
export type VariationLocale = "en" | "pt-BR";

export type VariationCopy = {
  brand: string;
  controlPlane: {
    activeRoutesLabel: string;
    badge: string;
    body: string;
    categoriesTitle: string;
    contextLabel: string;
    governanceTitle: string;
    integrationMeta: string;
    integrationNames: string;
    logStreamLabel: string;
    loggedLabel: string;
    observabilityTitle: string;
    payloadSourceLabel: string;
    protectedLabel: string;
    redactionTitle: string;
    requestIdLabel: string;
    routes: readonly {
      body: string;
      status: string;
      title: string;
    }[];
    title: string;
  };
  cta: {
    requestDemo: string;
    seeWorkflow: string;
  };
  desktop: {
    ariaLabel: string;
    briefBody: string;
    briefStatus: string;
    briefTitle: string;
    title: string;
  };
  footer: {
    copyright: string;
    description: string;
  };
  hero: {
    body: string;
    kicker: string;
    pricing: string;
    title: string;
  };
  inside: {
    kicker: string;
    title: string;
  };
  modal: {
    closeLabel: string;
    emailLabel: string;
    kicker: string;
    note: string;
    placeholder: string;
    success: string;
    title: string;
  };
  nav: {
    inside: string;
    setup: string;
    trusted: string;
  };
  report: {
    rows: readonly {
      label: string;
      meta: string;
      value: string;
    }[];
    summary: string;
    title: string;
    value: string;
  };
  steps: readonly {
    body: string;
    icon: ProcessIconName;
    num: string;
    percent?: number;
    suffix?: string;
    title: string;
  }[];
  cards: readonly [string, string, string][];
};

export const variationCopies = {
  en: {
    brand: "Cutokyo",
    controlPlane: {
      activeRoutesLabel: "4 live routes",
      badge: "native context orchestration",
      body: "Cutokyo connects agent hooks, shared memory, MCP tools and webhooks so the right context reaches every AI and automation natively — with local governance, security and observability built in.",
      categoriesTitle: "Native context flow",
      contextLabel: "Integration event",
      governanceTitle: "Native integrations",
      integrationMeta: "hooks + MCP + webhooks",
      integrationNames: "Claude · Codex · OpenCode",
      logStreamLabel: "Event stream",
      loggedLabel: "Active",
      observabilityTitle: "Event routing",
      payloadSourceLabel: "hook + webhook",
      protectedLabel: "native hooks active",
      redactionTitle: "Security",
      requestIdLabel: "Event ID",
      routes: [
        {
          body: "Capture live tasks, prompts and tool results directly from every connected agent.",
          status: "connected",
          title: "Agent hooks",
        },
        {
          body: "Reuse decisions and session history across agents without rebuilding context by hand.",
          status: "available",
          title: "Shared memory",
        },
        {
          body: "Make approved tools and resources available wherever the work is happening.",
          status: "synced",
          title: "MCP & tools",
        },
        {
          body: "Route governed events to automations and systems already in your stack.",
          status: "routed",
          title: "Webhooks",
        },
      ],
      title: "Put the right context to work across every AI integration.",
    },
    cta: {
      requestDemo: "Request demo",
      seeWorkflow: "See the workflow",
    },
    desktop: {
      ariaLabel: "Live savings breakdown",
      briefBody:
        "Cutokyo saved $428 this month while keeping your Claude, Codex, and OpenCode workflow unchanged.",
      briefStatus: "ready",
      briefTitle: "cost split",
      title: "Cutokyo Desktop · savings live",
    },
    footer: {
      copyright: "© 2026 Cutokyo. Local-first operations for AI agents.",
      description:
        "One local control plane for memory, security and observability across AI agents.",
    },
    hero: {
      body: "Search, resume, protect and understand your AI coding sessions across Claude, Codex, OpenCode and more — without abandoning the tools you already use.",
      kicker: "local-first agent operations",
      pricing: "Local SQLite · hooks first · desktop + terminal",
      title: "Take control of every\ntool in your stack.",
    },
    inside: {
      kicker: "the operating layer",
      title: "Everything your agents forget, expose or scatter — under control.",
    },
    modal: {
      closeLabel: "Close demo request",
      emailLabel: "Work email",
      kicker: "request demo",
      note: "Demo access is currently invite-only.",
      placeholder: "you@company.com",
      success: "Your email is now in the list.",
      title: "See Cutokyo on your own AI spend.",
    },
    nav: {
      inside: "Capabilities",
      setup: "How it works",
      trusted: "Integrations",
    },
    report: {
      rows: [
        { label: "Input tokens", meta: "before Cutokyo", value: "8.4m" },
        { label: "Optimized tokens", meta: "after Cutokyo", value: "2.5m" },
        { label: "Token reduction", meta: "less context", value: "70%" },
        { label: "Projected AI cost saved", meta: "blended models", value: "52%" },
        { label: "Estimated savings", meta: "this month", value: "$428" },
        { label: "Claude Code", meta: "cost down", value: "51%" },
        { label: "Codex", meta: "cost down", value: "48%" },
      ],
      summary: "8.4m input tokens → 2.5m optimized tokens. That is 70% less context sent.",
      title: "Tokens cut",
      value: "70%",
    },
    steps: [
      {
        body: "Install the lightweight desktop app or terminal package on Windows, macOS or Linux.",
        icon: "install",
        num: "01",
        title: "Connect your agents",
      },
      {
        body: "Hooks capture sessions, tools and usage automatically, with a proxy available as fallback.",
        icon: "auth",
        num: "02",
        title: "Work as usual",
      },
      {
        body: "Search the shared history, inspect what happened and resume any session with one click.",
        icon: "savings",
        num: "03",
        title: "Resume anywhere",
      },
    ],
    cards: [
      [
        "diff",
        "Session history & resume",
        "Search sessions across Claude Code, Codex and OpenCode. Reopen supported sessions in their original agent and project directory.",
      ],
      [
        "lock",
        "Local data control",
        "Keep session history on your machine. Choose how long to retain it, preview deletions, and create or restore local backups.",
      ],
      [
        "token",
        "Usage & capture health",
        "Compare recorded token usage and estimated costs across agents. Check capture and storage health, with missing data clearly identified.",
      ],
      [
        "spark",
        "Agent tools management",
        "Inspect skills, MCP servers, hooks, plugins and instruction files. Edit supported configurations and copy skills or compatible MCP servers between agents.",
      ],
    ],
  },
  "pt-BR": {
    brand: "Cutokyo",
    controlPlane: {
      activeRoutesLabel: "4 rotas ativas",
      badge: "orquestração nativa de contexto",
      body: "A Cutokyo conecta hooks de agentes, memória compartilhada, ferramentas MCP e webhooks para levar o contexto certo a cada IA e automação — com governança, segurança e observabilidade locais desde a origem.",
      categoriesTitle: "Fluxo nativo de contexto",
      contextLabel: "Evento de integração",
      governanceTitle: "Integrações nativas",
      integrationMeta: "hooks + MCP + webhooks",
      integrationNames: "Claude · Codex · OpenCode",
      logStreamLabel: "Fluxo de eventos",
      loggedLabel: "Ativo",
      observabilityTitle: "Roteamento de eventos",
      payloadSourceLabel: "hook + webhook",
      protectedLabel: "hooks nativos ativos",
      redactionTitle: "Segurança",
      requestIdLabel: "ID do evento",
      routes: [
        {
          body: "Capture tarefas, prompts e resultados de ferramentas direto de cada agente conectado.",
          status: "conectado",
          title: "Hooks de agentes",
        },
        {
          body: "Reutilize decisões e o histórico das sessões entre agentes sem remontar o contexto à mão.",
          status: "disponível",
          title: "Memória compartilhada",
        },
        {
          body: "Disponibilize ferramentas e recursos aprovados exatamente onde o trabalho acontece.",
          status: "sincronizado",
          title: "MCP e ferramentas",
        },
        {
          body: "Envie eventos governados às automações e aos sistemas que já fazem parte da operação.",
          status: "roteado",
          title: "Webhooks",
        },
      ],
      title: "Use o contexto certo em cada integração de IA.",
    },
    cta: {
      requestDemo: "Solicitar demo",
      seeWorkflow: "Ver o fluxo",
    },
    desktop: {
      ariaLabel: "Resumo de economia ao vivo",
      briefBody:
        "A Cutokyo economizou US$ 428 este mês mantendo seu fluxo no Claude, Codex e OpenCode sem mudanças.",
      briefStatus: "pronto",
      briefTitle: "divisão da economia",
      title: "Cutokyo Desktop · economia ao vivo",
    },
    footer: {
      copyright: "© 2026 Cutokyo. Operações locais para agentes de IA.",
      description: "Um painel local para memória, segurança e observabilidade entre agentes de IA.",
    },
    hero: {
      body: "Pesquise, retome, proteja e entenda suas sessões de programação com IA no Claude, Codex, OpenCode e outros — sem abandonar as ferramentas que você já usa.",
      kicker: "operações locais para agentes",
      pricing: "SQLite local · hooks primeiro · desktop + terminal",
      title: "Um painel de controle para todos os seus agentes de IA",
    },
    inside: {
      kicker: "a camada operacional",
      title: "Tudo o que seus agentes esquecem, expõem ou espalham — sob controle.",
    },
    modal: {
      closeLabel: "Fechar solicitação de demo",
      emailLabel: "Email de trabalho",
      kicker: "solicitar demo",
      note: "A demo está disponível apenas por convite no momento.",
      placeholder: "voce@empresa.com",
      success: "Seu email agora está na lista.",
      title: "Veja a Cutokyo no seu próprio gasto com IA.",
    },
    nav: {
      inside: "Recursos",
      setup: "Como funciona",
      trusted: "Integrações",
    },
    report: {
      rows: [
        { label: "Tokens de entrada", meta: "antes da Cutokyo", value: "8,4m" },
        { label: "Tokens otimizados", meta: "depois da Cutokyo", value: "2,5m" },
        { label: "Redução de tokens", meta: "menos contexto", value: "70%" },
        { label: "Custo de IA economizado", meta: "modelos combinados", value: "52%" },
        { label: "Economia estimada", meta: "este mês", value: "US$ 428" },
        { label: "Claude Code", meta: "custo menor", value: "51%" },
        { label: "Codex", meta: "custo menor", value: "48%" },
      ],
      summary:
        "8,4m tokens de entrada → 2,5m tokens otimizados. Isso é 70% menos contexto enviado.",
      title: "Tokens cortados",
      value: "70%",
    },
    steps: [
      {
        body: "Instale o app desktop leve ou o pacote para terminal no Windows, macOS ou Linux.",
        icon: "install",
        num: "01",
        title: "Conecte seus agentes",
      },
      {
        body: "Hooks capturam sessões, ferramentas e uso automaticamente, com proxy disponível como alternativa.",
        icon: "auth",
        num: "02",
        title: "Trabalhe normalmente",
      },
      {
        body: "Pesquise o histórico compartilhado, entenda o que aconteceu e retome qualquer sessão com um clique.",
        icon: "savings",
        num: "03",
        title: "Retome de onde quiser",
      },
    ],
    cards: [
      [
        "diff",
        "Histórico e retomada",
        "Pesquise sessões do Claude Code, Codex e OpenCode. Retome sessões compatíveis no agente e na pasta do projeto em que foram iniciadas.",
      ],
      [
        "lock",
        "Controle dos dados locais",
        "Mantenha o histórico de sessões na sua máquina. Defina por quanto tempo guardá-lo, revise exclusões e crie ou restaure backups locais.",
      ],
      [
        "token",
        "Uso e saúde da captura",
        "Compare o uso registrado de tokens e os custos estimados entre agentes. Confira a saúde da captura e do armazenamento, com dados ausentes sinalizados.",
      ],
      [
        "spark",
        "Gestão de ferramentas",
        "Inspecione skills, servidores MCP, hooks, plugins e arquivos de instruções. Edite configurações compatíveis e copie skills ou servidores MCP compatíveis entre agentes.",
      ],
    ],
  },
} as const satisfies Record<VariationLocale, VariationCopy>;
