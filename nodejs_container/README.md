# holochain-nodejs

Nodejs Holochain Container primarily for the execution of tests

## Installation

The recommended way to install is via npm https://www.npmjs.com/package/@holochain/holochain-nodejs.

To build from source clone the repo and run
```
node ./publish.js
```
from the project root.

## Usage
The following demo shows how to spin up two separate instances of a hApp, within the container.

After installing via npm the module can be used in a node script as follows:
```javascript
const dnaPath = "path/to/happ.hcpkg"
const aliceAgentId = "alice"
const tashAgentId = "tash"
// destructure to get Config and Container off the main import, which is an object now
const { Config, Container } = require('@holochain/holochain-nodejs')

// build up a configuration for the container, step by step
const agentAlice = Config.agent(aliceAgentId)
const agentTash = Config.agent(tashAgentId)
const dna = Config.dna(dnaPath)
const instanceAlice = Config.instance(agentAlice, dna)
const instanceTash = Config.instance(agentTash, dna)
const config = Config.container(instanceAlice, instanceTash)

// create a new instance of a Container, from the config
const container = new Container(config)

// this starts all the configured instances
container.start()

// When building up a config using `Config`, the instance ID is automatically assigned
// as the given agent ID plus a double colon plus the given dnaPath.
// We'll need this to call the instance later.
const aliceInstanceId = aliceAgentId + '::' + dnaPath

// zome functions can be called using the following
const callResult = container.call(aliceInstanceId, zome, capability, fnName, paramsAsObject)

// get the actual agent_id for an instance, by passing an instance id
const actualAliceAgentId = container.agent_id(aliceInstanceId)

// stop all running instances
container.stop()
```

container.start, container.call, container.agent_id, and container.stop are the four functions of Container instances currently.

Note about usage:
Prior to version ???, a container would only return a single instance of an app. Now a container actually contains multiple instances. When performing a call to an instance, one must include the instance id. Take the following for example:

```
const callResult = container.call(someInstanceId, someZome, someCapability, someFunction, someParams)
```

If you wanted to go on using the old syntax of individuating the apps, you could use the following
helper function which is exposed on `Container`:

```
const dnaPath = "path/to/happ.hcpkg"
...
const container = new Container(config)
const alice = container.makeCaller('alice', dnaPath)

// now you can use `alice` as a slightly more convenient way of calling this instance
// (the following four params would need to be replaced with valid values)
alice.call(someZome, someCapability, someFunction, someParams)

// you can also get the agent's address this way:
alice.agentId
```

## Deployment
Recommended pattern for deployment:

In your CLI, navigate to the directory containing these files.

Use `npm version [patch, minor, major]` (depending on the type of update)
This will update the package.json.

Commit this.

Push it to github.

Create a tag on github of the format `holochain-nodejs-vY.Y.Y` where `Y.Y.Y` is the version number of the tag. This is really important, as only a tag with this format will trigger release builds to happen. This is configured in the .travis.yml file.

This will cause the CI to build for all platforms, and upload the binaries to github releases.

If are added as a team member on the holochain team on npm, and have previously run `npm adduser`, skip this step.
If you haven't, run `npm adduser`.
Use the details of your npm user to login.

Once travis has finished with the binary uploads to releases (progress can be seen at https://travis-ci.org/holochain/holochain-rust) run the following from your computer, from the `nodejs_holochain` directory
`node ./publish.js --publish`

Until windows for travis can utilize secure environment variables without breaking (its not available as a feature yet), we cannot re-enable the automated npm publish step. When the time comes, the configuration is already in the travis file, commented out.

## Authors

- Julian Laubstein <contact@julianlaubstein.de>
- Connor Turland <connor.turland@holo.host>
- Willem Olding <willem.olding@holo.host>

## Acknowledgments

- Thanks to IronCoreLabs for the example of deploying neon modules via npm (https://github.com/IronCoreLabs/recrypt-node-binding)
