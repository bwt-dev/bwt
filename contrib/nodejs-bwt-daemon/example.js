const BwtDaemon = require('bwt-daemon')

;(async function(){
  const my_desc = 'wpkh(tpubD6NzVbkrYhZ4Ya1aR2od7JTGK6b44cwKhWzrvrTeTWFrzGokdAGHrZLK6BdYwpx9K7EoY38LzHva3SWwF8yRrXM9x9DQ3jCGKZKt1nQEz7n/0/*)';

  const bwtd = await BwtDaemon({
    network: 'regtest',
    bitcoind_dir: '/tmp/bd1',
    bitcoind_wallet: 'bwt',
    electrum_rpc_addr: '127.0.0.1:0',
    http_server_addr: '127.0.0.1:0',
    descriptors: [ [ my_desc, 'now' ] ],
    progress_cb: progress => console.log('bwt progress %f%%', progress*100),
    verbose: 2,
  })

  console.log('bwt running', bwtd.electrum_rpc_addr, bwtd.http_server_addr)

  // Connect to the HTTP API. Requires `npm install node-fetch`
  const fetch = require('node-fetch')
  const bwt = (...path) => fetch(bwtd.http_server_url + path.join('/')).then(r => r.json())

  console.log('wallets:', await bwt('wallets'))
  console.log('address:', await bwt('wallet/qufmgwfu/10'))
  console.log('transactions:', await bwt('txs/since/0'))

  setTimeout(_ => bwtd.shutdown(), 5000)
})()
