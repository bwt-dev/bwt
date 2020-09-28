from functools import partial
from xml.sax.saxutils import escape

from PyQt5.QtCore import Qt, QObject, pyqtSignal
from PyQt5.QtGui import QTextOption
from PyQt5.QtWidgets import QSizePolicy, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit, QTextEdit, QComboBox, QPushButton, QFormLayout

from electrum.i18n import _
from electrum.plugin import hook
from electrum.gui.qt.util import EnterButton, Buttons, CloseButton, WindowModalDialog

from .bwt import BwtPlugin

from electrum.logging import get_logger
_logger = get_logger('plugins.bwt')

class Plugin(BwtPlugin, QObject):

    log_signal = pyqtSignal(str, str, str)

    def __init__(self, *args):
        BwtPlugin.__init__(self, *args)
        QObject.__init__(self)
        self.parent.bwt = self
        self.closed = False

    def requires_settings(self):
        return True

    def settings_widget(self, window):
        return EnterButton(_('Connect to bitcoind'), partial(self.settings_dialog, window))

    def settings_dialog(self, window):
        # hack to workaround a bug: https://github.com/spesmilo/electrum/commit/4d8fcded4b42fd673bbb61f85aa99dc329be28a4
        if self.closed:
            # if this plugin instance is supposed to be closed and we have a different newer instance available,
            # forward the call to the newer one.
            if self.parent.bwt and self.parent.bwt != self:
                self.parent.bwt.settings_dialog(window)
            return

        if not self.wallets:
            window.show_error(_('No watch-only hd wallets found. Note that bwt cannot currently be used with hot wallets. See the README for more details.'))
            return

        d = WindowModalDialog(window, _('Connect to Bitcoin Core with bwt'))
        d.setMinimumWidth(570)
        vbox = QVBoxLayout(d)

        form = QFormLayout()
        form.setLabelAlignment(Qt.AlignRight | Qt.AlignVCenter)
        vbox.addLayout(form)

        form.addRow(title(_('Bitcoin Core settings')))

        url_e = input(self.bitcoind_url)
        form.addRow(_('RPC URL:'), url_e)

        auth_e = input(self.bitcoind_auth)
        auth_e.setPlaceholderText('<username>:<password>')
        form.addRow(_('RPC Auth:'), auth_e)
        form.addRow('', helptext(_('Leave blank to use the cookie.'), False))

        dir_e = input(self.bitcoind_dir)
        form.addRow(_('Directory:'), dir_e)
        form.addRow('', helptext(_('Used for reading the cookie file. Ignored if auth is set.'), False))

        wallet_e = input(self.bitcoind_wallet, 150)
        form.addRow(_('Wallet:'), wallet_e)
        form.addRow('', helptext(_('For use with multi-wallet. Leave blank to use the default wallet.'), False))


        form.addRow(title(_('Other settings')))

        rescan_c = QComboBox()
        rescan_c.addItems([ _('All history'), _('Since date'), _('None') ])
        rescan_c.setMaximumWidth(150)
        rescan_e = input(None, 150)
        rescan_e.setPlaceholderText('yyyy-mm-dd')
        apply_rescan(self.rescan_since, rescan_c, rescan_e)
        rescan_c.currentIndexChanged.connect(lambda i: rescan_e.setVisible(i == 1))
        rescan_l = QHBoxLayout()
        rescan_l.addWidget(rescan_c)
        rescan_l.addWidget(rescan_e)
        form.addRow(_('Scan:'), rescan_l)
        form.addRow('', helptext(_('Set to the wallet creation date to reduce scanning time.'), False))

        custom_opt_e = input(self.custom_opt)
        custom_opt_e.setPlaceholderText('e.g. --gap-limit 50 --poll-interval 1')
        form.addRow('Options', custom_opt_e)
        form.addRow('', helptext(_('Additional custom options. Optional.'), False))

        verbose_c = QComboBox()
        verbose_c.addItems([ _('info'), _('debug'), _('trace') ])
        verbose_c.setCurrentIndex(self.verbose)
        verbose_c.setMaximumWidth(150)
        form.addRow('Log level:', verbose_c)

        log_t = QTextEdit()
        log_t.setReadOnly(True)
        log_t.setFixedHeight(80)
        log_t.setStyleSheet('QTextEdit { color: #888; font-size: 0.9em }')
        log_t.setWordWrapMode(QTextOption.WrapAnywhere)
        sp = log_t.sizePolicy()
        sp.setRetainSizeWhenHidden(True)
        log_t.setSizePolicy(sp)
        log_t.hide()
        form.addRow(log_t)

        self.log_signal.connect(partial(show_log, log_t))

        def save_config_and_run():
            self.enabled = True
            self.bitcoind_url = str(url_e.text())
            self.bitcoind_dir = str(dir_e.text())
            self.bitcoind_auth = str(auth_e.text())
            self.bitcoind_wallet = str(wallet_e.text())
            self.rescan_since = get_rescan_value(rescan_c, rescan_e)
            self.custom_opt = str(custom_opt_e.text())
            self.verbose = verbose_c.currentIndex()

            self.config.set_key('bwt_enabled', self.enabled)
            self.config.set_key('bwt_bitcoind_url', self.bitcoind_url)
            self.config.set_key('bwt_bitcoind_dir', self.bitcoind_dir)
            self.config.set_key('bwt_bitcoind_auth', self.bitcoind_auth)
            self.config.set_key('bwt_bitcoind_wallet', self.bitcoind_wallet)
            self.config.set_key('bwt_rescan_since', self.rescan_since)
            self.config.set_key('bwt_custom_opt', self.custom_opt)
            self.config.set_key('bwt_verbose', self.verbose)

            log_t.clear()
            log_t.show()

            self.start()

            window.show_message(_('bwt is starting, check the logs for additional information. The bwt server will be available after Bitcoin Core completes rescanning, which may take awhile.'))
            log_t.ensureCursorVisible()

        save_b = QPushButton('Save && Connect')
        save_b.setDefault(True)
        save_b.clicked.connect(save_config_and_run)

        vbox.addLayout(Buttons(CloseButton(d), save_b))

        d.exec_()
        self.log_signal.disconnect()

    @hook
    def init_qt(self, gui_object):
        daemon = gui_object.daemon
        # `get_wallets()` in v4, `wallets` attr in v3
        wallets = daemon.get_wallets() if hasattr(daemon, 'get_wallets') else daemon.wallets
        for path, wallet in wallets.items():
            self.load_wallet(wallet, None)

    def handle_log(self, *log):
        BwtPlugin.handle_log(self, *log)
        self.log_signal.emit(*log)

    def close(self):
        BwtPlugin.close(self)
        self.closed = True
        self.parent.bwt = None

def title(text):
    l = QLabel(text)
    l.setStyleSheet('QLabel { font-weight: bold }')
    return l

def input(value=None, width=400):
    le = QLineEdit()
    if value is not None: le.setText(value)
    le.setMaximumWidth(width)
    return le

def helptext(text, wrap=True):
    l = QLabel(text)
    l.setWordWrap(wrap)
    l.setStyleSheet('QLabel { color: #aaa; font-size: 0.9em }')
    return l

def show_log(log_t, level, pkg, msg):
    scrollbar = log_t.verticalScrollBar()
    wasOnBottom = scrollbar.value() >= scrollbar.maximum() - 5

    color = { 'ERROR': '#CD0200', 'WARN': '#D47500', 'INFO': '#4BBF73', 'DEBUG': '#2780E3', 'TRACE': '#888'}.get(level, 'auto')
    frag = '<p><span style="color:%s">%s</span> <strong>%s</strong> » %s</p>' \
           % (color, escape(level), escape(pkg), escape(msg))
    log_t.append(frag)
    log_t.show()
    # scroll to end
    if wasOnBottom: log_t.ensureCursorVisible()

def apply_rescan(value, rescan_c, rescan_e):
    if value == "all":
        rescan_c.setCurrentIndex(0)
        rescan_e.hide()
    elif value == "none":
        rescan_c.setCurrentIndex(2)
        rescan_e.hide()
    else:
        rescan_c.setCurrentIndex(1)
        rescan_e.setText(value)
        rescan_e.show()

def get_rescan_value(rescan_c, rescan_e):
    index = rescan_c.currentIndex()
    if index == 0: return 'all'
    elif index == 1: return str(rescan_e.text())
    elif index == 2: return 'none'
