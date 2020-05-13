(async function () {
  const url = 'http://localhost:3060/'
      , bwt = (...path) => fetch(url + path.join('/')).then(r => r.json())
      , stream = new EventSource(url + 'stream')

  const utxos = await bwt('utxos')
      , balance = _ => utxos.reduce((total, utxo) => total + utxo.amount, 0)
      , showBalance = _ => console.log(`has ${utxos.length} utxos worth ${balance()} sats`)

  showBalance()

  const listeners = {
    ChainTip (block_height, block_hash) {
      console.log(`chain tip updated to ${block_height} ${block_hash}`)
    },

    async Transaction (txid, block_height) {
      const tx = await bwt('tx', txid)
      console.log('new wallet transaction:', tx)
    },

    async TxoCreated (outpoint, block_height) {
      const [ txid, vout ] = outpoint.split(':')
      utxos[outpoint] = await bwt('txo', txid, vout)
      console.log('new unspent wallet txo:', utxos[outpoint])
      showBalance()
    },

    TxoSpent (outpoint, inpoint, block_height) {
      delete utxos[outpoint]
      console.log(`wallet txo ${outpoint} spent by ${inpoint}`)
      showBalance()
    }
  }

  stream.addEventListener('message', msg => {
    const { category, params } = JSON.parse(msg.data)
    console.log('event:', category, ...params)
    if (listeners[category]) listeners[category](...params)
  })

})()
