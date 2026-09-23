/*
 * Bittorrent Client using Qt and libtorrent.
 * Copyright (C) 2006  Christophe Dumez <chris@qbittorrent.org>
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License
 * as published by the Free Software Foundation; either version 2
 * of the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301, USA.
 *
 * In addition, as a special exception, the copyright holders give permission to
 * link this program with the OpenSSL project's "OpenSSL" library (or with
 * modified versions of it that use the same license as the "OpenSSL" library),
 * and distribute the linked executables. You must obey the GNU General Public
 * License in all respects for all of the code used other than "OpenSSL".  If you
 * modify file(s), you may extend this exception to your version of the file(s),
 * but you are not obligated to do so. If you do not wish to do so, delete this
 * exception statement from your version.
 */

#include "statusbar.h"

#include <QDebug>
#include <QFrame>
#include <QHBoxLayout>
#include <QLabel>
#include <QPushButton>
#include <QStyle>

#include "base/bittorrent/session.h"
#include "base/bittorrent/sessionstatus.h"
#include "base/preferences.h"
#include "base/utils/misc.h"
#include "speedlimitdialog.h"
#include "uithememanager.h"
#include "utils.h"

namespace
{
    QWidget *createSeparator(QWidget *parent)
    {
        QFrame *separator = new QFrame(parent);
        separator->setFrameStyle(QFrame::VLine);
    #ifndef Q_OS_MACOS
        separator->setFrameShadow(QFrame::Raised);
    #endif
        return separator;
    }
}

StatusBar::StatusBar(QWidget *parent)
    : QStatusBar(parent)
{
#ifndef Q_OS_MACOS
    // Redefining global stylesheet breaks certain elements on mac like tabs.
    // Qt checks whether the stylesheet class inherits("QMacStyle") and this becomes false.
    setStyleSheet(u"QStatusBar::item { border-width: 0; }"_s);
#endif

    BitTorrent::Session *const session = BitTorrent::Session::instance();
    connect(session, &BitTorrent::Session::speedLimitModeChanged, this, &StatusBar::updateAltSpeedsBtn);
    QWidget *container = new QWidget(this);
    auto *layout = new QHBoxLayout(container);
    layout->setContentsMargins(0,0,0,0);

    container->setLayout(layout);
    m_dlSpeedLbl = new QPushButton(this);
    connect(m_dlSpeedLbl, &QAbstractButton::clicked, this, &StatusBar::capSpeed);
    m_dlSpeedLbl->setFlat(true);
    m_dlSpeedLbl->setFocusPolicy(Qt::NoFocus);
    m_dlSpeedLbl->setCursor(Qt::PointingHandCursor);
    m_dlSpeedLbl->setStyleSheet(u"text-align:left;"_s);
    m_dlSpeedLbl->setMinimumWidth(200);

    m_upSpeedLbl = new QPushButton(this);
    connect(m_upSpeedLbl, &QAbstractButton::clicked, this, &StatusBar::capSpeed);
    m_upSpeedLbl->setFlat(true);
    m_upSpeedLbl->setFocusPolicy(Qt::NoFocus);
    m_upSpeedLbl->setCursor(Qt::PointingHandCursor);
    m_upSpeedLbl->setStyleSheet(u"text-align:left;"_s);
    m_upSpeedLbl->setMinimumWidth(200);

    m_freeDiskSpaceLbl = new QLabel(this);
    m_freeDiskSpaceLbl->setSizePolicy(QSizePolicy::Maximum, QSizePolicy::Preferred);
    m_freeDiskSpaceSeparator = createSeparator(this);

    m_altSpeedsBtn = new QPushButton(this);
    m_altSpeedsBtn->setFlat(true);
    m_altSpeedsBtn->setFocusPolicy(Qt::NoFocus);
    m_altSpeedsBtn->setCursor(Qt::PointingHandCursor);
    connect(m_altSpeedsBtn, &QAbstractButton::clicked, this, &StatusBar::alternativeSpeedsButtonClicked);

    // Because on some platforms the default icon size is bigger
    // and it will result in taller/fatter statusbar, even if the
    // icons are actually 16x16
    m_dlSpeedLbl->setIconSize(Utils::Gui::smallIconSize());
    m_upSpeedLbl->setIconSize(Utils::Gui::smallIconSize());
    m_altSpeedsBtn->setIconSize(QSize(Utils::Gui::mediumIconSize().width(), Utils::Gui::smallIconSize().height()));

    // Set to the known maximum width(plus some padding)
    // so the speed widgets will take the rest of the space
    m_altSpeedsBtn->setMaximumWidth(Utils::Gui::largeIconSize().width());
    refreshIcons();
    connect(UIThemeManager::instance(), &UIThemeManager::themeChanged, this, &StatusBar::refreshIcons);

    layout->addWidget(m_freeDiskSpaceLbl);
    layout->addWidget(m_freeDiskSpaceSeparator);

    layout->addWidget(m_altSpeedsBtn);
    layout->addWidget(createSeparator(m_altSpeedsBtn));

    layout->addWidget(m_dlSpeedLbl);
    layout->addWidget(createSeparator(m_dlSpeedLbl));

    layout->addWidget(m_upSpeedLbl);

    addPermanentWidget(container);
    setStyleSheet(u"QWidget {margin: 0;}"_s);
    container->adjustSize();
    adjustSize();
    updateFreeDiskSpaceVisibility();
    refresh();
    connect(session, &BitTorrent::Session::statsUpdated, this, &StatusBar::refresh);

    updateFreeDiskSpaceLabel(session->freeDiskSpace());
    connect(session, &BitTorrent::Session::freeDiskSpaceChecked, this, &StatusBar::updateFreeDiskSpaceLabel);

    connect(Preferences::instance(), &Preferences::changed, this, &StatusBar::optionsSaved);
}

StatusBar::~StatusBar()
{
    qDebug() << Q_FUNC_INFO;
}

void StatusBar::showRestartRequired()
{
    // Restart required notification
    const QString restartText = tr("qbutt needs to be restarted!");

    const QPixmap pixmap = style()->standardIcon(QStyle::SP_MessageBoxWarning).pixmap(Utils::Gui::smallIconSize());
    auto *restartIconLbl = new QLabel(this);
    restartIconLbl->setPixmap(pixmap);
    restartIconLbl->setToolTip(restartText);
    insertWidget(0, restartIconLbl);

    auto *restartLbl = new QLabel(this);
    restartLbl->setText(restartText);
    insertWidget(1, restartLbl);
}

void StatusBar::updateFreeDiskSpaceLabel(const qint64 value)
{
    m_freeDiskSpaceLbl->setText((value >= 0)
        ? (tr("Free space: ") + Utils::Misc::friendlyUnit(value)) : tr("Free space unavailable"));
}

void StatusBar::updateFreeDiskSpaceVisibility()
{
    const bool isVisible = Preferences::instance()->isStatusbarFreeDiskSpaceDisplayed();
    m_freeDiskSpaceLbl->setVisible(isVisible);
    m_freeDiskSpaceSeparator->setVisible(isVisible);
}

void StatusBar::updateSpeedLabels()
{
    const BitTorrent::SessionStatus &sessionStatus = BitTorrent::Session::instance()->status();

    QString dlSpeedLbl = Utils::Misc::friendlyUnit(sessionStatus.payloadDownloadRate, true);
    const int dlSpeedLimit = BitTorrent::Session::instance()->downloadSpeedLimit();
    if (dlSpeedLimit > 0)
        dlSpeedLbl += u" [" + Utils::Misc::friendlyUnit(dlSpeedLimit, true) + u']';
    dlSpeedLbl += u" (" + Utils::Misc::friendlyUnit(sessionStatus.totalPayloadDownload) + u')';
    m_dlSpeedLbl->setText(dlSpeedLbl);

    QString upSpeedLbl = Utils::Misc::friendlyUnit(sessionStatus.payloadUploadRate, true);
    const int upSpeedLimit = BitTorrent::Session::instance()->uploadSpeedLimit();
    if (upSpeedLimit > 0)
        upSpeedLbl += u" [" + Utils::Misc::friendlyUnit(upSpeedLimit, true) + u']';
    upSpeedLbl += u" (" + Utils::Misc::friendlyUnit(sessionStatus.totalPayloadUpload) + u')';
    m_upSpeedLbl->setText(upSpeedLbl);
}

void StatusBar::refreshIcons()
{
    m_dlSpeedLbl->setIcon(UIThemeManager::instance()->getIcon(u"downloading"_s, u"downloading_small"_s));
    m_upSpeedLbl->setIcon(UIThemeManager::instance()->getIcon(u"upload"_s, u"seeding"_s));
    updateAltSpeedsBtn(BitTorrent::Session::instance()->isAltGlobalSpeedLimitEnabled());
}

void StatusBar::refresh()
{
    updateSpeedLabels();
}

void StatusBar::updateAltSpeedsBtn(bool alternative)
{
    if (alternative)
    {
        m_altSpeedsBtn->setIcon(UIThemeManager::instance()->getIcon(u"slow"_s));
        m_altSpeedsBtn->setToolTip(tr("Click to switch to regular speed limits"));
        m_altSpeedsBtn->setDown(true);
    }
    else
    {
        m_altSpeedsBtn->setIcon(UIThemeManager::instance()->getIcon(u"slow_off"_s));
        m_altSpeedsBtn->setToolTip(tr("Click to switch to alternative speed limits"));
        m_altSpeedsBtn->setDown(false);
    }
    refresh();
}

void StatusBar::capSpeed()
{
    auto *dialog = new SpeedLimitDialog {parentWidget()};
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    dialog->open();
}

void StatusBar::optionsSaved()
{
    updateFreeDiskSpaceVisibility();
}
