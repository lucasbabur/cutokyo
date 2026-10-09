import { tokenSavingEnabled } from "@/shared/config/features";

export type ProcessIconName = "install" | "auth" | "savings" | "govern";
export type MarketingLocale = "en" | "pt-BR";

export type MarketingCopy = {
  brand: string;
  controlPlane: {
    badge: string;
    body: string;
    categoriesTitle: string;
    contextLabel: string;
    governanceTitle: string;
    idleBody: string;
    idleTitle: string;
    inputPlaceholder: string;
    logStreamLabel: string;
    loggedLabel: string;
    observabilityTitle: string;
    redactionTitle: string;
    requestIdLabel: string;
    runLabel: string;
    title: string;
  };
  cta: {
    requestDemo: string;
    seeWorkflow: string;
  };
  download: {
    body: string;
    checksum: string;
    kicker: string;
    platforms: {
      appImage: string;
      deb: string;
      macArm: string;
      macIntel: string;
      windows: string;
    };
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
    contactLabel: string;
    kicker: string;
    note: string;
    title: string;
  };
  nav: {
    inside: string;
    setup: string;
    trusted: string;
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

const marketingCopies = {
  en: {
    brand: "Cutokyo",
    controlPlane: {
      badge: "live governance layer",
      body: "Paste real context to run a real backend analysis. Cutokyo redacts sensitive data, logs the analysis request to the local OpenTelemetry-shaped log stream, and shows where context is being spent.",
      categoriesTitle: "Context use",
      contextLabel: "Context payload",
      governanceTitle: "Governance",
      idleBody: "Enter context on the left. Nothing is submitted until you run the analysis.",
      idleTitle: "Ready",
      inputPlaceholder: "Paste model context to analyze…",
      logStreamLabel: "Log stream",
      loggedLabel: "Logged",
      observabilityTitle: "Observability",
      redactionTitle: "Security",
      requestIdLabel: "Request ID",
      runLabel: "Run live analysis",
      title: "See what your model context is being used on.",
    },
    cta: {
      requestDemo: "Request demo",
      seeWorkflow: "See the workflow",
    },
    download: {
      body: "Install Cutokyo once. The signed desktop connects supported AI tools and keeps observability, security, and controls local.",
      checksum: "SHA-256",
      kicker: "desktop downloads",
      platforms: {
        appImage: "Linux AppImage",
        deb: "Linux DEB",
        macArm: "macOS Apple Silicon",
        macIntel: "macOS Intel",
        windows: "Windows",
      },
      title: "Download Cutokyo",
    },
    footer: {
      copyright: "© 2026 Cutokyo. Built for cheaper model calls.",
      description:
        "Cut up to 70% of tokens and save ~50% on AI costs without changing your workflow.",
    },
    hero: {
      body: "Cutokyo is an efficiency layer between you and the model. Install it once, log in, and it makes your Claude, Codex, Gemini, and other agent calls cheaper by removing wasted tokens before they hit the model.",
      kicker: "desktop token compressor",
      pricing: "Built by researchers with published research in the AI space.",
      title: "Save ~50% on ai costs without changing your flow",
    },
    inside: {
      kicker: "what's inside",
      title: "Built for teams that want lower AI bills without extra work.",
    },
    modal: {
      closeLabel: "Close demo request",
      contactLabel: "Email Cutokyo",
      kicker: "request demo",
      note: "Demo access is currently invite-only. Contact us and we will reply from the real Cutokyo inbox.",
      title: "See Cutokyo on your own AI spend.",
    },
    nav: {
      inside: "What's inside",
      setup: "Setup process",
      trusted: "Trusted references",
    },
    steps: [
      {
        body: "Add the desktop app once and keep working in the agents you already use.",
        icon: "install",
        num: "01",
        title: "Install Cutokyo",
      },
      {
        body: "Sign in once and let Cutokyo work with Claude, Codex, Gemini, and other major agents.",
        icon: "auth",
        num: "02",
        title: "Log in",
      },
      {
        body: "Cut up to 70% of tokens and lower AI spend by ~50% without changing your flow.",
        icon: "savings",
        num: "03",
        percent: 70,
        suffix: "of tokens",
        title: "Save",
      },
    ],
    cards: [
      [
        "token",
        "Token compression meter",
        "Track the 70% token reduction and the ~50% AI cost savings before the request reaches a model.",
      ],
      [
        "diff",
        "Context analysis",
        "See which tools, files, messages, and model calls are spending the context budget.",
      ],
      [
        "lock",
        "Security redaction",
        "Redact secrets and sensitive values before context is analyzed or shown to teammates.",
      ],
      [
        "spark",
        "Governance logging",
        "Emit who-requested-what records to the customer-controlled local log layer.",
      ],
    ],
  },
  "pt-BR": {
    brand: "Cutokyo",
    controlPlane: {
      badge: "camada de governança ao vivo",
      body: "Cole um contexto real para rodar uma análise real no backend. A Cutokyo redige dados sensíveis, registra a solicitação de análise no fluxo local em formato OpenTelemetry e mostra onde o contexto está sendo gasto.",
      categoriesTitle: "Uso do contexto",
      contextLabel: "Payload de contexto",
      governanceTitle: "Governança",
      idleBody: "Insira o contexto à esquerda. Nada é enviado até você iniciar a análise.",
      idleTitle: "Pronto",
      inputPlaceholder: "Cole o contexto do modelo para analisar…",
      logStreamLabel: "Fluxo de logs",
      loggedLabel: "Registrado",
      observabilityTitle: "Observabilidade",
      redactionTitle: "Segurança",
      requestIdLabel: "ID da solicitação",
      runLabel: "Executar análise ao vivo",
      title: "Veja no que o contexto do modelo está sendo usado.",
    },
    cta: {
      requestDemo: "Solicitar demo",
      seeWorkflow: "Ver o fluxo",
    },
    download: {
      body: "Instale a Cutokyo uma vez. O desktop assinado conecta as ferramentas de IA compatíveis e mantém observabilidade, segurança e controles locais.",
      checksum: "SHA-256",
      kicker: "downloads desktop",
      platforms: {
        appImage: "Linux AppImage",
        deb: "Linux DEB",
        macArm: "macOS Apple Silicon",
        macIntel: "macOS Intel",
        windows: "Windows",
      },
      title: "Baixe a Cutokyo",
    },
    footer: {
      copyright: "© 2026 Cutokyo. Feito para chamadas de modelo mais baratas.",
      description: "Corte até 70% dos tokens e economize ~50% em custos de IA sem mudar seu fluxo.",
    },
    hero: {
      body: "A Cutokyo é uma camada de eficiência entre você e o modelo. Instale uma vez, faça login e deixe suas chamadas no Claude, Codex, Gemini e outros agentes mais baratas removendo tokens desperdiçados antes de chegarem ao modelo.",
      kicker: "compressor de tokens desktop",
      pricing: "Criada por pesquisadores com pesquisas publicadas na área de IA.",
      title: "Economize ~50% em custos de IA sem mudar seu fluxo",
    },
    inside: {
      kicker: "o que tem dentro",
      title: "Feito para times que querem contas de IA menores sem trabalho extra.",
    },
    modal: {
      closeLabel: "Fechar solicitação de demo",
      contactLabel: "Enviar email para a Cutokyo",
      kicker: "solicitar demo",
      note: "A demo está disponível apenas por convite. Entre em contato e responderemos pela caixa de entrada real da Cutokyo.",
      title: "Veja a Cutokyo no seu próprio gasto com IA.",
    },
    nav: {
      inside: "O que tem dentro",
      setup: "Processo de setup",
      trusted: "Referências",
    },
    steps: [
      {
        body: "Adicione o app desktop uma vez e continue trabalhando nos agentes que você já usa.",
        icon: "install",
        num: "01",
        title: "Instale a Cutokyo",
      },
      {
        body: "Faça login uma vez e deixe a Cutokyo trabalhar com Claude, Codex, Gemini e outros agentes principais.",
        icon: "auth",
        num: "02",
        title: "Faça login",
      },
      {
        body: "Corte até 70% dos tokens e reduza o gasto com IA em ~50% sem mudar seu fluxo.",
        icon: "savings",
        num: "03",
        percent: 70,
        suffix: "dos tokens",
        title: "Economize",
      },
    ],
    cards: [
      [
        "token",
        "Medidor de compressão",
        "Acompanhe a redução de 70% dos tokens e a economia de ~50% em IA antes da chamada chegar ao modelo.",
      ],
      [
        "diff",
        "Análise de contexto",
        "Veja quais ferramentas, arquivos, mensagens e chamadas de modelo gastam o orçamento de contexto.",
      ],
      [
        "lock",
        "Redação de segurança",
        "Redija segredos e valores sensíveis antes de analisar ou mostrar o contexto ao time.",
      ],
      [
        "spark",
        "Logs de governança",
        "Emita registros de quem pediu o quê para a camada local de logs controlada pelo cliente.",
      ],
    ],
  },
} as const satisfies Record<MarketingLocale, MarketingCopy>;

/**
 * Landing page copy for deployments that ship without token saving.
 *
 * Only the sections that make a token-reduction claim are overridden; the rest
 * of the page is shared with the default copy above.
 *
 * NOTE: this replacement positioning is a functional placeholder. The default
 * copy sells the product on token savings end to end, so this wording needs
 * marketing sign-off before it goes in front of customers.
 */
const copyWithoutTokenSaving = {
  en: {
    footer: {
      copyright: "© 2026 Cutokyo. Built for governed model calls.",
      description: "See where your context budget goes and control what leaves your machine.",
    },
    hero: {
      body: "Cutokyo is a governance layer between you and the model. Install it once, log in, and it shows what your Claude, Codex, Gemini, and other agent calls are spending context on while redacting sensitive data before it leaves your machine.",
      kicker: "desktop context governance",
      pricing: "Built by researchers with published research in the AI space.",
      title: "See and govern every ai call without changing your flow",
    },
    steps: [
      {
        body: "Add the desktop app once and keep working in the agents you already use.",
        icon: "install",
        num: "01",
        title: "Install Cutokyo",
      },
      {
        body: "Sign in once and let Cutokyo work with Claude, Codex, Gemini, and other major agents.",
        icon: "auth",
        num: "02",
        title: "Log in",
      },
      {
        body: "See where context is spent, redact sensitive values, and keep an auditable record of every call.",
        icon: "govern",
        num: "03",
        title: "Govern",
      },
    ],
    cards: [
      [
        "diff",
        "Context analysis",
        "See which tools, files, messages, and model calls are spending the context budget.",
      ],
      [
        "lock",
        "Security redaction",
        "Redact secrets and sensitive values before analyzing or showing context to the team.",
      ],
      [
        "spark",
        "Governance logs",
        "Emit a record of who asked for what to the local, customer-controlled log layer.",
      ],
    ],
  },
  "pt-BR": {
    footer: {
      copyright: "© 2026 Cutokyo. Feito para chamadas de modelo governadas.",
      description:
        "Veja para onde vai seu orçamento de contexto e controle o que sai da sua máquina.",
    },
    hero: {
      body: "O Cutokyo é uma camada de governança entre você e o modelo. Instale uma vez, faça login e veja no que suas chamadas de Claude, Codex, Gemini e outros agentes gastam contexto, com redação de dados sensíveis antes de saírem da sua máquina.",
      kicker: "governança de contexto no desktop",
      pricing: "Construído por pesquisadores com pesquisa publicada na área de IA.",
      title: "Veja e governe cada chamada de ia sem mudar seu fluxo",
    },
    steps: [
      {
        body: "Instale o aplicativo desktop uma vez e continue trabalhando nos agentes que você já usa.",
        icon: "install",
        num: "01",
        title: "Instale o Cutokyo",
      },
      {
        body: "Faça login uma vez e deixe o Cutokyo trabalhar com Claude, Codex, Gemini e outros grandes agentes.",
        icon: "auth",
        num: "02",
        title: "Faça login",
      },
      {
        body: "Veja onde o contexto é gasto, redija valores sensíveis e mantenha um registro auditável de cada chamada.",
        icon: "govern",
        num: "03",
        title: "Governe",
      },
    ],
    cards: [
      [
        "diff",
        "Análise de contexto",
        "Veja quais ferramentas, arquivos, mensagens e chamadas de modelo gastam o orçamento de contexto.",
      ],
      [
        "lock",
        "Redação de segurança",
        "Redija segredos e valores sensíveis antes de analisar ou mostrar o contexto ao time.",
      ],
      [
        "spark",
        "Logs de governança",
        "Emita registros de quem pediu o quê para a camada local de logs controlada pelo cliente.",
      ],
    ],
  },
} as const satisfies Record<MarketingLocale, Partial<MarketingCopy>>;

/** Landing page copy resolved against the deployment's feature switches. */
export const activeMarketingCopies: Record<MarketingLocale, MarketingCopy> = tokenSavingEnabled
  ? marketingCopies
  : {
      en: { ...marketingCopies.en, ...copyWithoutTokenSaving.en },
      "pt-BR": { ...marketingCopies["pt-BR"], ...copyWithoutTokenSaving["pt-BR"] },
    };
