import subprocess
import threading
import platform
import pathlib
import socket
from os import path

from electrum import constants
from electrum.plugin import BasePlugin, hook
from electrum.i18n import _
from electrum.util import UserFacingException
from electrum.logging import get_logger
from electrum.network import Network

_logger = get_logger('plugins.bwt')

bwt_bin = path.join(path.dirname(__file__), 'bwt')

if platform.system() == 'Windows':
    bwt_bin = '%s.exe' % bwt_bin

class BwtPlugin(BasePlugin):

    def __init__(self, parent, config, name):
        BasePlugin.__init__(self, parent, config, name)
        self.proc = None
        self.wallets = set()

        self.enabled = config.get('bwt_enabled')
        self.bitcoind_url = config.get('bwt_bitcoind_url', default_bitcoind_url())
        self.bitcoind_dir = config.get('bwt_bitcoind_dir', path.join(pathlib.Path.home(), '.bitcoin'))
        self.bitcoind_wallet = config.get('bwt_bitcoind_wallet')
        self.bitcoind_cred = config.get('bwt_bitcoind_cred')
        self.rescan_since = config.get('bwt_rescan_since', 'all')
        self.socket_path = config.get('bwt_socket_path', path.join(path.dirname(__file__), 'bwt-socket'))
        self.poll_interval = config.get('bwt_poll_interval', 5)
        self.verbose = config.get('bwt_verbose', 0)

        self.start()

    def start(self):
        if not self.enabled or not self.wallets:
            return

        self.rpc_port = free_port()

        args = [
            '--network', get_network_name(),
            '--bitcoind-url', self.bitcoind_url,
            '--bitcoind-dir', self.bitcoind_dir,
            '--electrum-rpc-addr', '127.0.0.1:%d' % self.rpc_port,
        ]

        if self.bitcoind_cred:
            args.extend([ '--bitcoind-cred', self.bitcoind_cred ])

        if self.bitcoind_wallet:
            args.extend([ '--bitcoind-wallet', self.bitcoind_wallet ])

        if platform.system() == 'Linux':
            args.extend([ '--unix-listener-path', self.socket_path ])

        for wallet in self.wallets:
            for xpub in wallet.get_master_public_keys():
                args.extend([ '--xpub', '%s:%s' % (xpub, self.rescan_since) ])

        for i in range(self.verbose):
            args.append('-v')

        self.stop()
        _logger.info('Starting bwt daemon')
        _logger.debug('bwt options: %s' % ' '.join(args))

        self.proc = subprocess.Popen([ bwt_bin ] + args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.thread = threading.Thread(target=proc_logger, args=(self.proc, self.handle_log), daemon=True)
        self.thread.start()

    def stop(self):
        if self.proc:
            _logger.info('Stopping bwt daemon')
            self.proc.terminate()
            self.proc = None
            self.thread = None

    def set_server(self):
        network = Network.get_instance()
        net_params = network.get_parameters()._replace(
            host='127.0.0.1',
            port=self.rpc_port,
            protocol='t',
            oneserver=True,
        )
        network.run_from_another_thread(network.set_parameters(net_params))

    @hook
    def load_wallet(self, wallet, main_window):
        if wallet.get_master_public_keys():
            num_wallets = len(self.wallets)
            self.wallets |= {wallet}
            if len(self.wallets) != num_wallets:
                self.start()
        else:
            _logger.warning('%s wallets are unsupported, skipping' % wallet.wallet_type)

    @hook
    def close_wallet(self, wallet):
        self.wallets -= {wallet}
        if not self.wallets:
            self.stop()

    def close(self):
        BasePlugin.close(self)
        self.stop()

    def handle_log(self, level, pkg, msg):
        if msg.startswith('Electrum RPC server running'):
            self.set_server()

def proc_logger(proc, log_handler):
    for line in iter(proc.stdout.readline, b''):
        line = line.decode('utf-8').strip()
        _logger.debug(line)

        if '::' in line and '>' in line:
            level, _, line = line.partition(' ')
            pkg, _, msg = line.partition('>')
            log_handler(level, pkg.strip(), msg.strip())
        elif line.lower().startswith('error: '):
            log_handler('ERROR', 'bwt', line[7:])
        else:
            log_handler('INFO', 'bwt', line)


def get_network_name():
    if constants.net == constants.BitcoinMainnet:
        return 'bitcoin'
    elif constants.net == constants.BitcoinTestnet:
        return 'testnet'
    elif constants.net == constants.BitcoinRegtest:
        return 'regtest'

    raise UserFacingException(_('Unsupported network {}').format(constants.net))

def default_bitcoind_url():
    return 'http://localhost:%d/' % \
      { 'bitcoin': 8332, 'testnet': 18332, 'regtest': 18443 }[get_network_name()]

def free_port():
    with socket.socket() as s:
        s.bind(('',0))
        return s.getsockname()[1]
