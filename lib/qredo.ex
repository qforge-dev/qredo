defmodule Qredo do
  @moduledoc """
  Installs and runs the platform-specific qredo executable used by `mix qredo`.
  """

  @source_root Path.expand("..", __DIR__)
  @version @source_root |> Path.join("VERSION") |> File.read!() |> String.trim()
  @repository "qforge-dev/qredo"

  @doc "Returns the Mix package and native release version."
  def version, do: @version

  @doc "Installs the native executable if needed and returns its path."
  def install!(opts \\ []) do
    case System.get_env("QREDO_BINARY_PATH") do
      path when is_binary(path) and path != "" -> explicit_binary!(path)
      _ -> install_managed!(opts)
    end
  end

  @doc "Runs qredo with the supplied arguments and returns its exit status."
  def run(args) when is_list(args) do
    binary = install!()

    {_output, status} =
      System.cmd(binary, args,
        into: IO.stream(:stdio, :line),
        stderr_to_stdout: false
      )

    status
  end

  @doc false
  def target_for(os_type, architecture) do
    architecture = to_string(architecture)

    case {os_type, architecture} do
      {{:unix, :darwin}, "aarch64" <> _} -> {:ok, "aarch64-apple-darwin"}
      {{:unix, :darwin}, "arm64" <> _} -> {:ok, "aarch64-apple-darwin"}
      {{:unix, :darwin}, "x86_64" <> _} -> {:ok, "x86_64-apple-darwin"}
      {{:unix, :linux}, "aarch64" <> rest} -> linux_target("aarch64", rest)
      {{:unix, :linux}, "arm64" <> rest} -> linux_target("aarch64", rest)
      {{:unix, :linux}, "x86_64" <> rest} -> linux_target("x86_64", rest)
      {{:win32, _}, "x86_64" <> _} -> {:ok, "x86_64-pc-windows-gnu"}
      _ -> {:error, unsupported_platform(os_type, architecture)}
    end
  end

  @doc false
  def artifact_name(release_id, target) do
    suffix = if String.contains?(target, "windows"), do: ".exe", else: ""
    "qredo-#{release_id}-#{target}#{suffix}"
  end

  @doc false
  def verify_checksum(body, manifest, artifact) do
    expected =
      manifest
      |> String.split("\n", trim: true)
      |> Enum.find_value(fn line ->
        case Regex.run(~r/^([0-9a-fA-F]{64})\s+\*?(.+)$/, line) do
          [_, digest, ^artifact] -> String.downcase(digest)
          _ -> nil
        end
      end)

    actual = :crypto.hash(:sha256, body) |> Base.encode16(case: :lower)

    cond do
      is_nil(expected) -> {:error, "checksum missing for #{artifact}"}
      expected != actual -> {:error, "checksum mismatch for #{artifact}"}
      true -> :ok
    end
  end

  defp install_managed!(opts) do
    target = target!()
    release = release_info()
    artifact = artifact_name(release.id, target)
    destination = cache_path(release.id, target)
    force? = Keyword.get(opts, :force, false)
    source? = Keyword.get(opts, :source, false) or System.get_env("QREDO_BUILD") == "source"

    cond do
      source? ->
        build_from_source!(destination, force?)

      File.regular?(destination) and not force? ->
        destination

      System.get_env("QREDO_OFFLINE") in ["1", "true"] ->
        Mix.raise("qredo executable is not cached and QREDO_OFFLINE is set")

      true ->
        download!(release.tag, artifact, destination)
    end
  end

  defp explicit_binary!(path) do
    expanded = Path.expand(path)

    if File.regular?(expanded) do
      expanded
    else
      Mix.raise("QREDO_BINARY_PATH does not point to a file: #{expanded}")
    end
  end

  defp target! do
    case target_for(:os.type(), :erlang.system_info(:system_architecture)) do
      {:ok, target} -> target
      {:error, message} -> Mix.raise(message <> "; set QREDO_BUILD=source to compile locally")
    end
  end

  defp linux_target(arch, rest) do
    if String.contains?(rest, "musl") do
      {:error, "unsupported qredo platform #{arch}-linux-musl"}
    else
      {:ok, "#{arch}-unknown-linux-gnu"}
    end
  end

  defp unsupported_platform(os_type, architecture) do
    "unsupported qredo platform #{inspect(os_type)} / #{architecture}"
  end

  defp release_info do
    case System.get_env("QREDO_RELEASE_TAG") do
      nil -> inferred_release_info()
      "" -> inferred_release_info()
      tag -> %{tag: tag, id: System.get_env("QREDO_RELEASE_ID") || tag}
    end
  end

  defp inferred_release_info do
    stable = "v#{@version}"

    cond do
      exact_git_tag() == stable -> %{tag: stable, id: stable}
      sha = git_sha() -> %{tag: "dev", id: sha}
      true -> %{tag: stable, id: stable}
    end
  end

  defp exact_git_tag do
    with true <- git_checkout?(),
         {tag, 0} <- git(["describe", "--tags", "--exact-match", "--match", "v*", "HEAD"]) do
      String.trim(tag)
    else
      _ -> nil
    end
  end

  defp git_sha do
    with true <- git_checkout?(),
         {sha, 0} <- git(["rev-parse", "HEAD"]) do
      String.trim(sha)
    else
      _ -> nil
    end
  end

  defp git_checkout? do
    File.exists?(Path.join(@source_root, ".git")) and not is_nil(System.find_executable("git"))
  end

  defp git(args), do: System.cmd("git", ["-C", @source_root | args], stderr_to_stdout: true)

  defp cache_path(release_id, target) do
    mix_home =
      System.get_env("MIX_HOME") ||
        Path.join(System.user_home!(), ".mix")

    executable = if String.contains?(target, "windows"), do: "qredo.exe", else: "qredo"
    Path.join([mix_home, "qredo", release_id, target, executable])
  end

  defp download!(tag, artifact, destination) do
    base =
      System.get_env("QREDO_DOWNLOAD_BASE_URL") ||
        "https://github.com/#{@repository}/releases/download"

    release_url = "#{String.trim_trailing(base, "/")}/#{tag}"
    Mix.shell().info("Installing qredo #{tag} for #{target!()}")
    manifest = fetch!("#{release_url}/SHA256SUMS")
    body = fetch!("#{release_url}/#{artifact}")

    case verify_checksum(body, manifest, artifact) do
      :ok -> write_executable!(destination, body)
      {:error, message} -> Mix.raise(message)
    end

    destination
  end

  defp fetch!(url) do
    {:ok, _} = Application.ensure_all_started(:inets)
    {:ok, _} = Application.ensure_all_started(:ssl)

    ssl = [
      verify: :verify_peer,
      cacerts: :public_key.cacerts_get(),
      customize_hostname_check: [match_fun: :public_key.pkix_verify_hostname_match_fun(:https)]
    ]

    headers = [{~c"user-agent", ~c"qredo-mix/#{@version}"}]
    request = {String.to_charlist(url), headers}

    case :httpc.request(:get, request, [autoredirect: true, ssl: ssl], body_format: :binary) do
      {:ok, {{_, status, _}, _, body}} when status in 200..299 ->
        body

      {:ok, {{_, status, reason}, _, _}} ->
        Mix.raise("download failed (#{status} #{reason}): #{url}")

      {:error, reason} ->
        Mix.raise("download failed (#{inspect(reason)}): #{url}")
    end
  end

  defp write_executable!(destination, body) do
    File.mkdir_p!(Path.dirname(destination))
    temporary = destination <> ".tmp-#{System.unique_integer([:positive])}"
    File.write!(temporary, body, [:binary])
    File.chmod!(temporary, 0o755)
    File.rename!(temporary, destination)
  end

  defp build_from_source!(destination, force?) do
    if File.regular?(destination) and not force? do
      destination
    else
      cargo =
        System.find_executable("cargo") || Mix.raise("cargo is required for QREDO_BUILD=source")

      Mix.shell().info("Building qredo #{@version} from source")

      {_output, status} =
        System.cmd(cargo, ["build", "--release", "--locked", "-p", "qredo"],
          cd: @source_root,
          into: IO.stream(:stdio, :line),
          stderr_to_stdout: true
        )

      if status != 0, do: Mix.raise("cargo failed to build qredo")

      executable = if match?({:win32, _}, :os.type()), do: "qredo.exe", else: "qredo"
      source = Path.join([@source_root, "target", "release", executable])
      write_executable!(destination, File.read!(source))
      destination
    end
  end
end
