const BwtDaemon = require('.')

;(async function(){
  const my_desc = 'wpkh(tpubD6NzVbkrYhZ4Ya1aR2od7JTGK6b44cwKhWzrvrTeTWFrzGokdAGHrZLK6BdYwpx9K7EoY38LzHva3SWwF8yRrXM9x9DQ3jCGKZKt1nQEz7n/0/*)';

  const bwt = await BwtDaemon({
    network: 'regtest',
    bitcoind_dir: '/tmp/bd1',
    bitcoind_wallet: 'bwt',
    electrum_rpc_addr: '127.0.0.1:0',
    http_server_addr: '127.0.0.1:0',
    descriptors: [ [ my_desc, 'now' ] ],
    progress_cb: progress => console.log('bwt progress %f%%', progress*100),
    verbose: 2,
  })

  console.log('bwt running', bwt.electrum_rpc_addr, bwt.http_server_addr)

  setTimeout(_ => bwt.shutdown(), 5000)
})()
