const repositoryApiUrl = "https://api.github.com/repos/lucasbabur/cutokyo";

export async function getCutokyoGitHubStars(signal: AbortSignal): Promise<number> {
  const response = await fetch(repositoryApiUrl, {
    cache: "no-cache",
    signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
  });
  if (!response.ok) throw new Error(`GitHub repository request failed with ${response.status}`);

  const data: { stargazers_count?: unknown } = await response.json();
  if (
    typeof data.stargazers_count !== "number" ||
    !Number.isSafeInteger(data.stargazers_count) ||
    data.stargazers_count < 0
  ) {
    throw new Error("GitHub returned an invalid star count");
  }

  return data.stargazers_count;
}
