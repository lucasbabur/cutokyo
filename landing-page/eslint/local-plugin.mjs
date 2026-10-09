const FUNCTION_TYPES = new Set([
  "ArrowFunctionExpression",
  "FunctionDeclaration",
  "FunctionExpression",
]);

const ARITHMETIC_OPERATORS = new Set(["*", "/", "%"]);
const COLLECTION_METHODS = new Set(["reduce", "sort"]);
const COMPLEXITY_FILE_PATTERN = /[/\\]src[/\\](app|features|entities)[/\\].+\.(ts|tsx)$/;
const EXEMPT_FILE_PATTERN = /(?:\.test|\.spec)\.(?:ts|tsx)$|(?:^|[/\\])(schema|types)\.ts$/;
const SKIPPED_KEYS = new Set(["comments", "loc", "parent", "range", "tokens"]);

function isNode(value) {
  return Boolean(value) && typeof value === "object" && typeof value.type === "string";
}

function countComplexity(rootNode) {
  let score = 0;
  let statements = 0;

  const visit = (node, isRoot = false) => {
    if (!node || typeof node !== "object") {
      return;
    }

    if (Array.isArray(node)) {
      node.forEach((item) => visit(item));
      return;
    }

    if (isNode(node)) {
      if (!isRoot && FUNCTION_TYPES.has(node.type)) {
        return;
      }

      if (node.type.endsWith("Statement")) {
        statements += 1;
      }

      switch (node.type) {
        case "IfStatement":
        case "ConditionalExpression":
        case "TryStatement":
          score += 1;
          break;
        case "DoWhileStatement":
        case "ForInStatement":
        case "ForOfStatement":
        case "ForStatement":
        case "SwitchStatement":
        case "WhileStatement":
          score += 2;
          break;
        default:
          break;
      }

      if (node.type === "BinaryExpression" && ARITHMETIC_OPERATORS.has(node.operator)) {
        score += 1;
      }

      if (
        node.type === "CallExpression" &&
        node.callee.type === "MemberExpression" &&
        !node.callee.computed &&
        node.callee.property.type === "Identifier" &&
        COLLECTION_METHODS.has(node.callee.property.name)
      ) {
        score += 1;
      }
    }

    Object.entries(node).forEach(([key, value]) => {
      if (SKIPPED_KEYS.has(key)) {
        return;
      }

      visit(value);
    });
  };

  visit(rootNode.body ?? rootNode, true);

  return {
    score,
    statements,
  };
}

function getFunctionName(node) {
  if (node.type === "FunctionDeclaration" && node.id?.name) {
    return node.id.name;
  }

  const parent = node.parent;

  if (parent?.type === "VariableDeclarator" && parent.id.type === "Identifier") {
    return parent.id.name;
  }

  if (parent?.type === "Property" && parent.key.type === "Identifier") {
    return parent.key.name;
  }

  return "function";
}

const localPlugin = {
  rules: {
    "no-complex-business-logic": {
      meta: {
        docs: {
          description:
            "Keep complex business logic out of client-heavy frontend modules and move it to Firebase or FastAPI.",
        },
        schema: [
          {
            additionalProperties: false,
            properties: {
              maxScore: {
                minimum: 0,
                type: "number",
              },
              maxStatements: {
                minimum: 1,
                type: "number",
              },
            },
            type: "object",
          },
        ],
        type: "suggestion",
      },
      create(context) {
        const filename = context.filename ?? context.getFilename();

        if (!COMPLEXITY_FILE_PATTERN.test(filename) || EXEMPT_FILE_PATTERN.test(filename)) {
          return {};
        }

        const maxScore = context.options[0]?.maxScore ?? 4;
        const maxStatements = context.options[0]?.maxStatements ?? 18;

        const reportIfNeeded = (node) => {
          const { score, statements } = countComplexity(node);

          if (score <= maxScore && statements <= maxStatements) {
            return;
          }

          context.report({
            data: {
              maxScore: String(maxScore),
              maxStatements: String(maxStatements),
              name: getFunctionName(node),
              score: String(score),
              statements: String(statements),
            },
            message:
              "`{{name}}` looks like business logic (score {{score}}/{{maxScore}}, statements {{statements}}/{{maxStatements}}). Move orchestration to Firebase or FastAPI and keep frontend modules thin.",
            node,
          });
        };

        return {
          ArrowFunctionExpression: reportIfNeeded,
          FunctionDeclaration: reportIfNeeded,
          FunctionExpression: reportIfNeeded,
        };
      },
    },
  },
};

export default localPlugin;
