const BwtDaemon = require('bwt-daemon')

;(async function(){
  const my_desc = 'wpkh(tpubD6NzVbkrYhZ4Ya1aR2od7JTGK6b44cwKhWzrvrTeTWFrzGokdAGHrZLK6BdYwpx9K7EoY38LzHva3SWwF8yRrXM9x9DQ3jCGKZKt1nQEz7n/0/*)';

  const bwtd = await BwtDaemon({
    network: 'regtest',
    bitcoind_dir: '/tmp/bd1',
    bitcoind_wallet: 'bwt',
    descriptors: [ my_desc ],
    electrum: true,
    http: true,
    verbose: 2,
    progress: (type, progress, detail) => console.log('bwt progress %s %f%%', type, progress*100, detail),
  }).start()

  console.log('bwt running', bwtd.electrum_addr, bwtd.http_addr)

  // Connect to the HTTP API. Requires `npm install node-fetch`
  const fetch = require('node-fetch')
  const bwt = (...path) => fetch(bwtd.http_url + path.join('/')).then(r => r.json())

  console.log('wallets:', await bwt('wallets'))
  console.log('address:', await bwt('wallet/qufmgwfu/10'))
  console.log('transactions:', await bwt('txs'))

  setTimeout(_ => bwtd.shutdown(), 5000)
})()
.catch(console.error)
