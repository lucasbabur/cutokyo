import { strict as assert } from "node:assert";
import { test } from "node:test";

import {
  createTrustedKeyboard,
  type TrustedKeyboardIO,
} from "./trusted-keyboard";

function world() {
  const environment: NodeJS.ProcessEnv = {
    CUTOKYO_NATIVE_APPLICATION: "/private/package/usr/bin/cutokyo-desktop",
    CUTOKYO_NATIVE_COMPOSITOR_RUNTIME: "/host/compositor-runtime",
    HYPRLAND_INSTANCE_SIGNATURE: "synthetic-instance",
    HOME: "/private/home",
    XDG_CONFIG_HOME: "/private/config",
    XDG_DATA_HOME: "/private/data",
    XDG_CACHE_HOME: "/private/cache",
    XDG_RUNTIME_DIR: "/private/runtime",
  };
  const target = { pid: 41, address: "0xAbC1", mapped: true, xwayland: true };
  const foreign = { pid: 42, address: "0xabc2", mapped: true, xwayland: false };
  const state = {
    entries: ["self", "41", "42"],
    executables: new Map<number, string | Error>([
      [41, environment.CUTOKYO_NATIVE_APPLICATION!],
      [42, "/foreign/usr/bin/cutokyo-desktop"],
    ]),
    clients: [target, foreign],
    active: { ...target },
    acknowledgement: "ok\n",
    afterDispatch: () => {},
    commands: [] as { args: readonly string[]; env: NodeJS.ProcessEnv }[],
  };
  const io: TrustedKeyboardIO = {
    processes: () => state.entries,
    executable: (pid) => {
      const executable = state.executables.get(pid);
      if (executable instanceof Error) throw executable;
      if (executable === undefined)
        throw Object.assign(new Error("process disappeared"), {
          code: "ENOENT",
        });
      return executable;
    },
    hyprctl: (args, env) => {
      state.commands.push({ args: [...args], env: { ...env } });
      if (args[0] === "clients") return JSON.stringify(state.clients);
      if (args[0] === "activewindow") return JSON.stringify(state.active);
      assert.deepEqual(args.slice(0, 2), ["dispatch", "sendshortcut"]);
      state.afterDispatch();
      return state.acknowledgement;
    },
  };
  const dispatches = () =>
    state.commands.filter((command) => command.args[0] === "dispatch");
  return { environment, target, foreign, state, io, dispatches };
}

function refusesBeforeDispatch(
  change: (fixture: ReturnType<typeof world>) => void,
  reason: RegExp,
) {
  const fixture = world();
  change(fixture);
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  assert.throws(() => keyboard.send("Tab"), reason);
  assert.deepEqual(fixture.dispatches(), []);
  const queries = fixture.state.commands.length;
  assert.throws(
    () => keyboard.send("Return"),
    /previous ownership\/input failure/u,
  );
  assert.equal(fixture.state.commands.length, queries);
}

test("dispatches only scoped Tab and Return and keeps app HOME/XDG private", () => {
  const fixture = world();
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  const tab = keyboard.send("Tab");
  const enter = keyboard.send("Return");
  assert.deepEqual(
    fixture.dispatches().map((command) => command.args),
    [
      ["dispatch", "sendshortcut", ", Tab, address:0xAbC1"],
      ["dispatch", "sendshortcut", ", Return, address:0xAbC1"],
    ],
  );
  for (const command of fixture.state.commands) {
    assert.equal(command.env.XDG_RUNTIME_DIR, "/host/compositor-runtime");
    for (const name of [
      "HOME",
      "XDG_CONFIG_HOME",
      "XDG_DATA_HOME",
      "XDG_CACHE_HOME",
    ])
      assert.equal(command.env[name], fixture.environment[name]);
    assert.equal(command.env.HYPRLAND_INSTANCE_SIGNATURE, "synthetic-instance");
  }
  assert.equal(fixture.environment.XDG_RUNTIME_DIR, "/private/runtime");
  assert.deepEqual(tab.activeBefore, tab.activeAfter);
  assert.deepEqual(enter.activeBefore, enter.activeAfter);
  assert.equal(keyboard.dispatches.length, 2);
  assert.equal(
    fixture.state.commands.filter((command) => command.args[0] === "clients")
      .length,
    4,
  );
});

for (const missing of [
  "HYPRLAND_INSTANCE_SIGNATURE",
  "CUTOKYO_NATIVE_COMPOSITOR_RUNTIME",
  "CUTOKYO_NATIVE_APPLICATION",
]) {
  test(`unsupported/missing ${missing} blocks without IO or a skip`, () => {
    const fixture = world();
    delete fixture.environment[missing];
    assert.throws(
      () => createTrustedKeyboard(fixture.environment, fixture.io),
      /keyboard blocked/u,
    );
    assert.deepEqual(fixture.state.commands, []);
  });
}

test("private runtime cannot stand in for the original host compositor runtime", () => {
  const fixture = world();
  fixture.environment.CUTOKYO_NATIVE_COMPOSITOR_RUNTIME =
    fixture.environment.XDG_RUNTIME_DIR;
  assert.throws(
    () => createTrustedKeyboard(fixture.environment, fixture.io),
    /keyboard blocked/u,
  );
  assert.deepEqual(fixture.state.commands, []);
});

test("zero exact executable matches cannot select a basename-matching decoy", () => {
  refusesBeforeDispatch(
    ({ state }) => state.executables.set(41, "/source/debug/cutokyo-desktop"),
    /exactly one PID/u,
  );
});

test("two exact executable matches refuse all keys", () => {
  refusesBeforeDispatch(
    ({ environment, state }) =>
      state.executables.set(42, environment.CUTOKYO_NATIVE_APPLICATION!),
    /exactly one PID/u,
  );
});

test("process-exit and foreign permission races do not select another executable", () => {
  const fixture = world();
  fixture.state.entries.push("43", "44");
  fixture.state.executables.set(
    44,
    Object.assign(new Error("foreign process"), { code: "EACCES" }),
  );
  createTrustedKeyboard(fixture.environment, fixture.io).send("Tab");
  assert.equal(fixture.dispatches().length, 1);
});

test("unexpected /proc IO errors fail closed", () => {
  refusesBeforeDispatch(
    ({ state }) =>
      state.executables.set(
        42,
        Object.assign(new Error("proc IO failure"), { code: "EIO" }),
      ),
    /proc IO failure/u,
  );
});

test("missing owned window refuses dispatch", () => {
  refusesBeforeDispatch(({ state, foreign }) => {
    state.clients = [foreign];
  }, /exactly one compositor window/u);
});

test("multiple owned windows refuse dispatch", () => {
  refusesBeforeDispatch(
    ({ state, target }) => state.clients.push({ ...target, address: "0xabc3" }),
    /exactly one compositor window/u,
  );
});

test("unmapped owned window refuses dispatch", () => {
  refusesBeforeDispatch(({ target }) => {
    target.mapped = false;
  }, /not mapped/u);
});

test("duplicate compositor address refuses dispatch", () => {
  refusesBeforeDispatch(({ foreign, target }) => {
    foreign.address = target.address;
  }, /ambiguous compositor window address/u);
});

for (const address of [
  "abc1",
  "0x",
  "0xabc1\n",
  "0xabc1,all",
  "0xabc1;focuswindow",
  "0xg123",
]) {
  test(`invalid address ${JSON.stringify(address)} cannot reach sendshortcut`, () => {
    refusesBeforeDispatch(({ target }) => {
      target.address = address;
    }, /invalid compositor window address/u);
  });
}

test("foreign active XWayland window refuses dispatch", () => {
  refusesBeforeDispatch(({ state, foreign }) => {
    state.active = { ...foreign, xwayland: true };
  }, /different XWayland window/u);
});

test("different active XWayland address is refused even with the same PID", () => {
  refusesBeforeDispatch(({ state }) => {
    state.active.address = "0xabc3";
  }, /different XWayland window/u);
});

test("foreign Wayland active window is preserved without a focus request", () => {
  const fixture = world();
  fixture.state.active = { ...fixture.foreign };
  const trace = createTrustedKeyboard(fixture.environment, fixture.io).send(
    "Tab",
  );
  assert.deepEqual(trace.activeBefore, fixture.foreign);
  assert.deepEqual(trace.activeAfter, fixture.foreign);
  assert.equal(fixture.dispatches().length, 1);
});

test("active window change after dispatch blocks every subsequent key and retains trace", () => {
  const fixture = world();
  fixture.state.afterDispatch = () => {
    fixture.state.active = { ...fixture.foreign };
  };
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  assert.throws(
    () => keyboard.send("Tab"),
    /changed the active compositor window/u,
  );
  assert.deepEqual(keyboard.dispatches[0]?.activeAfter, fixture.foreign);
  assert.throws(
    () => keyboard.send("Return"),
    /previous ownership\/input failure/u,
  );
  assert.equal(fixture.dispatches().length, 1);
});

test("ownership is revalidated after each dispatch", () => {
  const fixture = world();
  fixture.state.afterDispatch = () => {
    fixture.state.executables.delete(41);
  };
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  assert.throws(() => keyboard.send("Tab"), /exactly one PID/u);
  assert.throws(
    () => keyboard.send("Return"),
    /previous ownership\/input failure/u,
  );
  assert.equal(fixture.dispatches().length, 1);
});

for (const identity of ["PID", "address"]) {
  test(`ownership ${identity} change between Tab and Return refuses a second dispatch`, () => {
    const fixture = world();
    const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
    keyboard.send("Tab");
    if (identity === "PID") {
      fixture.state.executables.delete(41);
      fixture.state.executables.set(
        43,
        fixture.environment.CUTOKYO_NATIVE_APPLICATION!,
      );
      fixture.state.entries.push("43");
      fixture.target.pid = 43;
    } else fixture.target.address = "0xabc3";
    assert.throws(() => keyboard.send("Return"), /ownership changed/u);
    assert.equal(fixture.dispatches().length, 1);
  });
}

test("dispatcher refusal is a failure, never a fallback to other input", () => {
  const fixture = world();
  fixture.state.acknowledgement = "not accepted";
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  assert.throws(() => keyboard.send("Tab"), /did not acknowledge/u);
  assert.throws(
    () => keyboard.send("Return"),
    /previous ownership\/input failure/u,
  );
  assert.equal(fixture.dispatches().length, 1);
});

test("owned mapping is revalidated after dispatch before allowing another key", () => {
  const fixture = world();
  fixture.state.afterDispatch = () => {
    fixture.target.mapped = false;
  };
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  assert.throws(() => keyboard.send("Tab"), /not mapped/u);
  assert.throws(
    () => keyboard.send("Return"),
    /previous ownership\/input failure/u,
  );
  assert.equal(fixture.dispatches().length, 1);
});

test("missing hyprctl is a blocked failure rather than a skipped journey", () => {
  refusesBeforeDispatch(({ io }) => {
    io.hyprctl = () => {
      throw Object.assign(new Error("hyprctl unavailable"), { code: "ENOENT" });
    };
  }, /keyboard blocked.*hyprctl unavailable/u);
});

test("malformed compositor JSON fails before dispatch", () => {
  refusesBeforeDispatch(({ io }) => {
    io.hyprctl = () => "not JSON";
  }, /keyboard blocked/u);
});

test("runtime key allowlist rejects any non-Tab/Return value", () => {
  const fixture = world();
  const keyboard = createTrustedKeyboard(fixture.environment, fixture.io);
  assert.throws(() => keyboard.send("Escape" as "Tab"), /only Tab and Return/u);
  assert.deepEqual(fixture.state.commands, []);
});
