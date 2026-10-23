import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const [engineRootArg, loaderRootArg] = process.argv.slice(2);
if (!engineRootArg) {
  console.error("Usage: node --import tsx check-world-config.mjs <engine-root> [loader-root]");
  process.exit(2);
}

const engineRoot = path.resolve(engineRootArg);
const loaderRoot = path.resolve(loaderRootArg ?? engineRootArg);
const configPath = path.resolve(engineRoot, "data/config/world.json");
const loaderPath = path.join(loaderRoot, "src/util/WorldConfig.ts");

if (!fs.existsSync(loaderPath)) {
  console.error(`Engine Q config check failed: loader not found at ${loaderPath}`);
  process.exit(1);
}
if (!fs.existsSync(configPath)) {
  console.error(`Engine Q config check failed: config not found at ${configPath}`);
  process.exit(1);
}
const expectedPath = fs.realpathSync(configPath);

process.chdir(engineRoot);
try {
  const { getWorldConfigPath, loadWorldConfig } = await import(pathToFileURL(loaderPath).href);
  const loadedPath = getWorldConfigPath();
  const world = loadWorldConfig();
  const observed = {
    webPort: world.web.port,
    managementPort: world.web.managementPort,
    gamePort: world.node.port,
    maxConnected: world.node.maxConnected,
    dbBackend: world.db.backend,
    dbName: world.db.name,
  };
  const expected = {
    webPort: 1180,
    managementPort: 9988,
    gamePort: 44694,
    maxConnected: 1,
    dbBackend: "sqlite",
    dbName: "lostcity-289-q",
  };
  const matches = Object.entries(expected).every(([key, value]) => observed[key] === value);
  if (loadedPath !== expectedPath || !matches) {
    console.error(JSON.stringify({ expectedPath, loadedPath, expected, observed }));
    process.exitCode = 1;
  } else {
    console.log(JSON.stringify({ configPath: loadedPath, ...observed }));
  }
} catch (error) {
  console.error(`Engine Q config check failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
