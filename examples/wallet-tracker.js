(async function () {

  // Simple client using fetch() and EventSource, both available on modern browsers and nodejs.

  const url = 'http://localhost:3060/'
      , bwt = (...path) => fetch(url + path.join('/')).then(r => r.json())
      , stream = new EventSource(url + 'stream')

  stream.addEventListener('message', msg => {
    const { category, params } = JSON.parse(msg.data)
    console.log(`event # ${category} # `, params)
    if (listeners[category]) listeners[category](...params)
  })

  // Create a map of utxos indexed by the txid:vout
  const makeUtxoMap = utxos =>
    utxos.reduce((M, utxo) => ({ ...M, [`${utxo.txid}:${utxo.vout}`]: utxo }), {})

  // App code

  const utxos = await bwt('utxos').then(makeUtxoMap)
      , balance = _ => Object.values(utxos).reduce((total, utxo) => total + utxo.amount, 0)
      , showBalance = _ => console.log(`has ${Object.keys(utxos).length} utxos worth ${balance()} sats`)

  showBalance()

  const listeners = {
    ChainTip (block_height, block_hash) {
      console.log(`chain tip updated to ${block_height} ${block_hash}`)
    },

    async Transaction (txid, block_height) {
      const tx = await bwt('tx', txid)
      console.log('new wallet transaction:', tx)
    },

    TxoFunded (outpoint, scripthash, amount, block_height) {
      console.log(`new unspent txo ${outpoint} worth ${amount} sats`)
      utxos[outpoint] = { amount, scripthash, block_height }
      showBalance()
    },

    TxoSpent (inpoint, scripthash, outpoint, height) {
      console.log(`wallet txo ${outpoint} spent by ${inpoint}`)
      delete utxos[outpoint]
      showBalance()
    },
  }
})()
