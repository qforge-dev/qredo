defmodule QredoTest do
  use ExUnit.Case, async: true

  test "R-MIX-1 maps supported hosts to release targets" do
    assert Qredo.target_for({:unix, :darwin}, "aarch64-apple-darwin") ==
             {:ok, "aarch64-apple-darwin"}

    assert Qredo.target_for({:unix, :darwin}, "x86_64-apple-darwin") ==
             {:ok, "x86_64-apple-darwin"}

    assert Qredo.target_for({:unix, :linux}, "x86_64-pc-linux-gnu") ==
             {:ok, "x86_64-unknown-linux-gnu"}

    assert Qredo.target_for({:unix, :linux}, "aarch64-unknown-linux-gnu") ==
             {:ok, "aarch64-unknown-linux-gnu"}

    assert Qredo.target_for({:win32, :nt}, "x86_64-pc-windows-msvc") ==
             {:ok, "x86_64-pc-windows-gnu"}

    assert {:error, message} =
             Qredo.target_for({:unix, :linux}, "riscv64-unknown-linux-gnu")

    assert message =~ "unsupported qredo platform"
  end

  test "R-MIX-2 names immutable release assets" do
    assert Qredo.artifact_name("v0.1.0", "aarch64-apple-darwin") ==
             "qredo-v0.1.0-aarch64-apple-darwin"

    assert Qredo.artifact_name("abc123", "x86_64-pc-windows-gnu") ==
             "qredo-abc123-x86_64-pc-windows-gnu.exe"
  end

  test "R-MIX-3 verifies the selected artifact checksum" do
    body = "native-binary"
    digest = :crypto.hash(:sha256, body) |> Base.encode16(case: :lower)
    manifest = "#{digest}  qredo-dev-x86_64-unknown-linux-gnu\n"

    assert :ok =
             Qredo.verify_checksum(
               body,
               manifest,
               "qredo-dev-x86_64-unknown-linux-gnu"
             )

    assert {:error, message} =
             Qredo.verify_checksum(
               "tampered",
               manifest,
               "qredo-dev-x86_64-unknown-linux-gnu"
             )

    assert message =~ "checksum mismatch"
  end
end
