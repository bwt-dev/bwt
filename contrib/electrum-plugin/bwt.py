import subprocess
import threading
import platform
import socket
import os
import re

from electrum import constants
from electrum.bip32 import BIP32Node
from electrum.plugin import BasePlugin, hook
from electrum.i18n import _
from electrum.util import UserFacingException
from electrum.logging import get_logger
from electrum.network import Network

# Introduced in Electrum v4
try: from electrum.interface import ServerAddr
except: pass

_logger = get_logger('plugins.bwt')

plugin_dir = os.path.dirname(__file__)

bwt_bin = os.path.join(plugin_dir, 'bwt')
if platform.system() == 'Windows':
    bwt_bin = '%s.exe' % bwt_bin

class BwtPlugin(BasePlugin):
    wallets = set()
    proc = None
    prev_settings = None

    def __init__(self, parent, config, name):
        BasePlugin.__init__(self, parent, config, name)

        self.enabled = config.get('bwt_enabled')
        self.bitcoind_url = config.get('bwt_bitcoind_url', default_bitcoind_url())
        self.bitcoind_dir = config.get('bwt_bitcoind_dir', default_bitcoind_dir())
        self.bitcoind_wallet = config.get('bwt_bitcoind_wallet')
        self.bitcoind_auth = config.get('bwt_bitcoind_auth', config.get('bwt_bitcoind_cred'))
        self.rescan_since = config.get('bwt_rescan_since', 'all')
        self.custom_opt = config.get('bwt_custom_opt')
        self.socket_path = config.get('bwt_socket_path', default_socket_path())
        self.verbose = config.get('bwt_verbose', 0)

        if self.enabled:
            self.set_config()
            self.start()

    def start(self):
        if not self.enabled or not self.wallets:
            return

        self.stop()

        self.rpc_port = free_port()

        args = [
            '--network', get_network_name(),
            '--bitcoind-url', self.bitcoind_url,
            '--bitcoind-dir', self.bitcoind_dir,
            '--rescan-since', self.rescan_since,
            '--electrum-addr', '127.0.0.1:%d' % self.rpc_port,
            '--electrum-skip-merkle',
            '--no-startup-banner',
        ]

        if self.bitcoind_auth:
            args.extend([ '--bitcoind-auth', self.bitcoind_auth ])

        if self.bitcoind_wallet:
            args.extend([ '--bitcoind-wallet', self.bitcoind_wallet ])

        if self.socket_path:
            args.extend([ '--unix-listener-path', self.socket_path ])

        for wallet in self.wallets:
            if wallet.m is None:
                xpub = wallet.get_master_public_key()
                args.extend([ '--xpub', xpub ])
            else: # Multisig wallet
                for desc in get_multisig_descriptors(wallet):
                    args.extend([ '--descriptor', desc ])

        for i in range(self.verbose):
            args.append('-v')

        if self.custom_opt:
            # XXX this doesn't support arguments with spaces. thankfully bwt doesn't currently have any.
            args.extend(self.custom_opt.split(' '))

        self.set_config()

        _logger.info('Starting the bwt daemon')
        _logger.debug('bwt options: %s' % ' '.join(args))

        if platform.system() == 'Windows':
            # hide the console window. can be done with subprocess.CREATE_NO_WINDOW in python 3.7.
            suinfo = subprocess.STARTUPINFO()
            suinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        else: suinfo = None

        self.proc = subprocess.Popen([ bwt_bin ] + args, startupinfo=suinfo, \
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        self.thread = threading.Thread(target=proc_logger, args=(self.proc, self.handle_log), daemon=True)
        self.thread.start()

    def stop(self):
        if self.proc:
            _logger.info('Stopping the bwt daemon')
            self.proc.terminate()
            self.proc = None
            self.thread = None

    # enable oneserver/skipmerklecheck and disable manual server selection
    def set_config(self):
        if self.prev_settings: return # run once

        self.prev_settings = { setting: self.config.cmdline_options.get(setting)
                               for setting in [ 'oneserver', 'skipmerklecheck', 'server' ] }

        # setting `oneserver`/`skipmerklecheck` directly on `cmdline_options` keeps the settings in-memory only without
        # persisting them to the config file, reducing the chance of accidentally leaving them on with public servers.
        self.config.cmdline_options['oneserver'] = True

        # for `skipmerklecheck`, this is also the only way to set it an runtime prior to v4 (see https://github.com/spesmilo/electrum/commit/61ccc1ccd3a437d98084089f1d4014ba46c96e3b)
        self.config.cmdline_options['skipmerklecheck'] = True

        # set a dummy server so electrum won't attempt connecting to other servers on startup. setting this
        # in `cmdline_options` also prevents the user from switching servers using the gui, which further reduces
        # the chance of accidentally connecting to public servers with inappropriate settings.
        self.config.cmdline_options['server'] = '127.0.0.1:1:t'

    def set_server(self):
        _logger.info('Configuring server to 127.0.0.1:%s', self.rpc_port)

        # first, remove the `server` config to allow `set_parameters()` below to update it and trigger the connection mechanism
        del self.config.cmdline_options['server']

        network = Network.get_instance()
        net_params = network.get_parameters()
        try:
            # Electrum v4
            server = ServerAddr('127.0.0.1', self.rpc_port, protocol='t')
            net_params = net_params._replace(server=server, oneserver=True)
        except:
            # Electrum v3
            net_params = net_params._replace(
                host='127.0.0.1',
                port=self.rpc_port,
                protocol='t',
                oneserver=True,
            )
        network.run_from_another_thread(network.set_parameters(net_params))

        # now set the server in `cmdline_options` to lock it in
        self.config.cmdline_options['server'] = '127.0.0.1:%s:t' % self.rpc_port

    @hook
    def load_wallet(self, wallet, main_window):
        if not wallet.get_master_public_keys():
            _logger.warning('skipping unsupported wallet type %s' % wallet.wallet_type)
        elif wallet.can_export():
            _logger.warning('skipping hot wallet')
        else:
            num_wallets = len(self.wallets)
            self.wallets |= {wallet}
            if len(self.wallets) != num_wallets:
                self.start()

    @hook
    def close_wallet(self, wallet):
        self.wallets -= {wallet}
        if not self.wallets:
            self.stop()

    def close(self):
        BasePlugin.close(self)
        self.stop()

        # restore the user's previous settings when the plugin is disabled
        if self.prev_settings:
            for setting, prev_value in self.prev_settings.items():
                if prev_value is None: self.config.cmdline_options.pop(setting, None)
                else: self.config.cmdline_options[setting] = prev_value
            self.prev_settings = None

    def handle_log(self, level, pkg, msg):
        if msg.startswith('Electrum RPC server running'):
            self.set_server()

def proc_logger(proc, log_handler):
    for line in iter(proc.stdout.readline, b''):
        line = line.decode('utf-8').strip()
        _logger.debug(line)

        m = re.match(r"^(?:\d{4}-[\d-]+T[\d:.]+Z )?(ERROR|WARN|INFO|DEBUG|TRACE) +([^ ]+) +> (.*)", line)

        if m is not None:
            log_handler(*m.groups())
        elif line.lower().startswith('error: '):
            log_handler('ERROR', 'bwt', line[7:])
        else:
            log_handler('INFO', 'bwt', line)

DESCRIPTOR_MAP_SH = {
    'p2sh': 'sh(%s)',
    'p2wsh': 'wsh(%s)',
    'p2wsh-p2sh': 'sh(wsh(%s))',
}

def get_multisig_descriptors(wallet):
    descriptor_fmt = DESCRIPTOR_MAP_SH[wallet.txin_type]
    if not descriptor_fmt:
        _logger.warn('missing descriptor type for %s' % wallet.txin_type)
        return ()

    xpubs = [convert_to_std_xpub(xpub) for xpub in wallet.get_master_public_keys()]
    def get_descriptor(child_index):
        desc_keys = ['%s/%d/*' % (xpub, child_index) for xpub in xpubs]
        return descriptor_fmt % 'sortedmulti(%d,%s)' % (wallet.m, ','.join(desc_keys))

    # one for receive, one for change
    return (get_descriptor(0), get_descriptor(1))

# Convert SLIP32 ypubs/zpubs into standard BIP32 xpubs
def convert_to_std_xpub(xpub):
    return BIP32Node.from_xkey(xpub) \
      ._replace(xtype='standard') \
      .to_xpub()

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

def default_bitcoind_dir():
    if platform.system() == 'Windows':
        return os.path.expandvars('%APPDATA%\\Bitcoin')
    elif platform.system() == 'Darwin':
        return os.path.expandvars('$HOME/Library/Application Support/Bitcoin')
    else:
        return os.path.expandvars('$HOME/.bitcoin')

def default_socket_path():
    if platform.system() in ('Linux', 'Darwin') and os.access(plugin_dir, os.W_OK | os.X_OK):
        return os.path.join(plugin_dir, 'bwt-socket')

def free_port():
    with socket.socket() as s:
        s.bind(('',0))
        return s.getsockname()[1]
