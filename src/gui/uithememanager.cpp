/*
 * Bittorrent Client using Qt and libtorrent.
 * Copyright (C) 2023-2024  Vladimir Golovnev <glassez@yandex.ru>
 * Copyright (C) 2019, 2021  Prince Gupta <jagannatharjun11@gmail.com>
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
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301,
 * USA.
 *
 * In addition, as a special exception, the copyright holders give permission to
 * link this program with the OpenSSL project's "OpenSSL" library (or with
 * modified versions of it that use the same license as the "OpenSSL" library),
 * and distribute the linked executables. You must obey the GNU General Public
 * License in all respects for all of the code used other than "OpenSSL".  If
 * you modify file(s), you may extend this exception to your version of the
 * file(s), but you are not obligated to do so. If you do not wish to do so,
 * delete this exception statement from your version.
 */

#include "uithememanager.h"

#include <QApplication>
#include <QFile>
#include <QFont>
#include <QFontDatabase>
#include <QPalette>
#include <QPixmapCache>
#include <QResource>
#include <QStyle>
#include <QStyleHints>

#include "base/global.h"
#include "base/logger.h"
#include "base/path.h"
#include "base/profile.h"
#include "base/preferences.h"
#include "uithemecommon.h"

namespace
{
    Path resolveThemePath(const Path &themePath)
    {
        return (themePath.isAbsolute() ? themePath : (Profile::instance()->rootPath() / themePath));
    }
}

UIThemeManager *UIThemeManager::m_instance = nullptr;

void UIThemeManager::freeInstance()
{
    delete m_instance;
    m_instance = nullptr;
}

void UIThemeManager::initInstance()
{
    if (!m_instance)
        m_instance = new UIThemeManager;
}

UIThemeManager::UIThemeManager()
    : m_useCustomTheme {Preferences::instance()->useCustomUITheme()}
#ifdef QBT_HAS_COLORSCHEME_OPTION
    , m_colorSchemeSetting {u"Appearance/ColorScheme"_s}
    , m_activeColorScheme {m_colorSchemeSetting.get(ColorScheme::System)}
#endif
#if (defined(Q_OS_UNIX) && !defined(Q_OS_MACOS))
    , m_useSystemIcons {Preferences::instance()->useSystemIcons()}
#endif
{
#ifdef Q_OS_WIN
    QApplication::setStyle(u"Fusion"_s);
    QFont font = qApp->font();
    if (QFontDatabase::hasFamily(u"Segoe UI Variable"_s))
        font.setFamily(u"Segoe UI Variable"_s);
    font.setPointSize(10);
    qApp->setFont(font);
#endif

#ifdef QBT_HAS_COLORSCHEME_OPTION
    applyColorScheme();
#endif

    // NOTE: Qt::QueuedConnection can be omitted as soon as support for Qt 6.5 is dropped
    connect(QApplication::styleHints(), &QStyleHints::colorSchemeChanged, this, &UIThemeManager::onColorSchemeChanged, Qt::QueuedConnection);

    if (m_useCustomTheme)
    {
        const Path themePath = resolveThemePath(Preferences::instance()->customUIThemePath());

        if (themePath.hasExtension(u".qbtheme"_s))
        {
            if (QResource::registerResource(themePath.data(), u"/uitheme"_s))
                m_themeSource = std::make_unique<QRCThemeSource>();
            else
                LogMsg(tr("Failed to load UI theme from file: \"%1\"").arg(themePath.toString()), Log::WARNING);
        }
        else if (themePath.filename() == CONFIG_FILE_NAME)
        {
            m_themeSource = std::make_unique<FolderThemeSource>(themePath.parentPath());
        }
    }

    if (!m_themeSource)
        m_themeSource = std::make_unique<DefaultThemeSource>();

    m_appliedColorMode = activeColorMode();
    applyPalette();
    applyStyleSheet();
}

UIThemeManager *UIThemeManager::instance()
{
    return m_instance;
}

#ifdef QBT_HAS_COLORSCHEME_OPTION
ColorScheme UIThemeManager::colorScheme() const
{
    return m_colorSchemeSetting.get(ColorScheme::System);
}

void UIThemeManager::previewColorScheme(const ColorScheme value)
{
    if (value == m_activeColorScheme)
        return;

    m_activeColorScheme = value;
    applyColorScheme();
    onColorSchemeChanged();
}

void UIThemeManager::setColorScheme(const ColorScheme value)
{
    if (value != colorScheme())
        m_colorSchemeSetting = value;
    previewColorScheme(value);
}

void UIThemeManager::applyColorScheme() const
{
    switch (m_activeColorScheme)
    {
    case ColorScheme::System:
    default:
        qApp->styleHints()->unsetColorScheme();
        break;
    case ColorScheme::Light:
        qApp->styleHints()->setColorScheme(Qt::ColorScheme::Light);
        break;
    case ColorScheme::Dark:
        qApp->styleHints()->setColorScheme(Qt::ColorScheme::Dark);
        break;
    }
}
#endif

ColorMode UIThemeManager::activeColorMode() const
{
#ifdef QBT_HAS_COLORSCHEME_OPTION
    if (m_activeColorScheme == ColorScheme::Dark)
        return ColorMode::Dark;
    if (m_activeColorScheme == ColorScheme::Light)
        return ColorMode::Light;
#endif
    return (qApp->styleHints()->colorScheme() == Qt::ColorScheme::Dark) ? ColorMode::Dark : ColorMode::Light;
}

void UIThemeManager::applyStyleSheet() const
{
    if (m_useCustomTheme)
    {
        qApp->setStyleSheet(QString::fromUtf8(m_themeSource->readStyleSheet()));
        return;
    }

    QFile styleSheet {u":/themes/builtin.qss"_s};
    if (!styleSheet.open(QIODevice::ReadOnly))
    {
        LogMsg(tr("Failed to load the built-in theme."), Log::WARNING);
        return;
    }
    qApp->setStyleSheet(QString::fromUtf8(styleSheet.readAll()));
}

void UIThemeManager::onColorSchemeChanged()
{
    const ColorMode colorMode = activeColorMode();
    if (colorMode == m_appliedColorMode)
        return;

    m_appliedColorMode = colorMode;
    // workaround to refresh styled controls once color scheme is changed
    qApp->setStyleSheet({});
    QApplication::setStyle(QApplication::style()->name());
    applyPalette();
    applyStyleSheet();
    emit themeChanged();
}

QIcon UIThemeManager::getIcon(const QString &iconId, [[maybe_unused]] const QString &fallback) const
{
    const ColorMode colorMode = m_appliedColorMode;
    auto &icons = (colorMode == ColorMode::Dark) ? m_darkModeIcons : m_icons;

    const auto iter = icons.find(iconId);
    if (iter != icons.end())
        return *iter;

#if (defined(Q_OS_UNIX) && !defined(Q_OS_MACOS))
    // Don't cache system icons because users might change them at run time
    if (m_useSystemIcons)
    {
        auto icon = QIcon::fromTheme(iconId);
        if (icon.isNull() || icon.availableSizes().isEmpty())
            icon = QIcon::fromTheme(fallback, QIcon(m_themeSource->getIconPath(iconId, colorMode).data()));
        return icon;
    }
#endif

    const QIcon icon {m_themeSource->getIconPath(iconId, colorMode).data()};
    icons[iconId] = icon;
    return icon;
}

QIcon UIThemeManager::getFlagIcon(const QString &countryIsoCode) const
{
    if (countryIsoCode.isEmpty())
        return {};

    const QString key = countryIsoCode.toLower();
    const auto iter = m_flags.constFind(key);
    if (iter != m_flags.cend())
        return *iter;

    const QIcon icon {u":/icons/flags/" + key + u".svg"};
    m_flags[key] = icon;
    return icon;
}

QPixmap UIThemeManager::getScaledPixmap(const QString &iconId, const int height) const
{
    // (workaround) svg images require the use of `QIcon()` to load and scale losslessly,
    // otherwise other image classes will convert it to pixmap first and follow-up scaling will become lossy.

    Q_ASSERT(height > 0);

    const QString cacheKey = u"uitheme:"_s + iconId + u'@' + QString::number(height)
            + ((m_appliedColorMode == ColorMode::Dark) ? u":dark"_s : u":light"_s);

    QPixmap pixmap;
    if (!QPixmapCache::find(cacheKey, &pixmap))
    {
        pixmap = getIcon(iconId).pixmap(height);
        QPixmapCache::insert(cacheKey, pixmap);
    }

    return pixmap;
}

QColor UIThemeManager::getColor(const QString &id) const
{
    const QColor color = m_themeSource->getColor(id, m_appliedColorMode);
    return color;
}

void UIThemeManager::applyPalette() const
{
    if (!m_useCustomTheme)
    {
        QPalette palette;
        const bool dark = (m_appliedColorMode == ColorMode::Dark);
        const QColor window = dark ? QColor(u"#202123"_s) : QColor(u"#f5f6f7"_s);
        const QColor base = dark ? QColor(u"#191a1c"_s) : QColor(u"#ffffff"_s);
        const QColor alternateBase = dark ? QColor(u"#25272a"_s) : QColor(u"#f1f3f5"_s);
        const QColor button = dark ? QColor(u"#2b2d30"_s) : QColor(u"#ffffff"_s);
        const QColor text = dark ? QColor(u"#e8eaed"_s) : QColor(u"#20252b"_s);
        const QColor disabledText = dark ? QColor(u"#858b92"_s) : QColor(u"#858c94"_s);
        const QColor border = dark ? QColor(u"#3a3e43"_s) : QColor(u"#d9dde2"_s);

        palette.setColor(QPalette::Window, window);
        palette.setColor(QPalette::Base, base);
        palette.setColor(QPalette::AlternateBase, alternateBase);
        palette.setColor(QPalette::Button, button);
        palette.setColor(QPalette::ToolTipBase, button);
        for (const QPalette::ColorRole role : {QPalette::WindowText, QPalette::Text,
            QPalette::ButtonText, QPalette::ToolTipText})
        {
            palette.setColor(role, text);
            palette.setColor(QPalette::Disabled, role, disabledText);
        }
        palette.setColor(QPalette::BrightText, Qt::white);
        palette.setColor(QPalette::PlaceholderText, dark ? QColor(u"#9aa0a7"_s) : QColor(u"#727b84"_s));
        palette.setColor(QPalette::Highlight, dark ? QColor(u"#2b4557"_s) : QColor(u"#dceefa"_s));
        palette.setColor(QPalette::HighlightedText, text);
        palette.setColor(QPalette::Disabled, QPalette::Highlight, alternateBase);
        palette.setColor(QPalette::Disabled, QPalette::HighlightedText, disabledText);
        palette.setColor(QPalette::Link, dark ? QColor(u"#009df7"_s) : QColor(u"#0879b9"_s));
        palette.setColor(QPalette::LinkVisited, dark ? QColor(u"#82caff"_s) : QColor(u"#536da3"_s));
        palette.setColor(QPalette::Light, dark ? QColor(u"#51565b"_s) : QColor(u"#c1c9d1"_s));
        palette.setColor(QPalette::Midlight, dark ? QColor(u"#34373b"_s) : QColor(u"#e9edf0"_s));
        palette.setColor(QPalette::Mid, border);
        palette.setColor(QPalette::Dark, dark ? QColor(u"#17191b"_s) : QColor(u"#d0d5db"_s));
        palette.setColor(QPalette::Shadow, dark ? QColor(u"#101113"_s) : QColor(u"#aeb5bd"_s));
        qApp->setPalette(palette);
        return;
    }

    struct ColorDescriptor
    {
        QString id;
        QPalette::ColorRole colorRole;
        QPalette::ColorGroup colorGroup;
    };

    const ColorDescriptor paletteColorDescriptors[] =
    {
        {u"Palette.Window"_s, QPalette::Window, QPalette::Normal},
        {u"Palette.WindowText"_s, QPalette::WindowText, QPalette::Normal},
        {u"Palette.Base"_s, QPalette::Base, QPalette::Normal},
        {u"Palette.AlternateBase"_s, QPalette::AlternateBase, QPalette::Normal},
        {u"Palette.Text"_s, QPalette::Text, QPalette::Normal},
        {u"Palette.ToolTipBase"_s, QPalette::ToolTipBase, QPalette::Normal},
        {u"Palette.ToolTipText"_s, QPalette::ToolTipText, QPalette::Normal},
        {u"Palette.BrightText"_s, QPalette::BrightText, QPalette::Normal},
        {u"Palette.Highlight"_s, QPalette::Highlight, QPalette::Normal},
        {u"Palette.HighlightedText"_s, QPalette::HighlightedText, QPalette::Normal},
        {u"Palette.Button"_s, QPalette::Button, QPalette::Normal},
        {u"Palette.ButtonText"_s, QPalette::ButtonText, QPalette::Normal},
        {u"Palette.Link"_s, QPalette::Link, QPalette::Normal},
        {u"Palette.LinkVisited"_s, QPalette::LinkVisited, QPalette::Normal},
        {u"Palette.Light"_s, QPalette::Light, QPalette::Normal},
        {u"Palette.Midlight"_s, QPalette::Midlight, QPalette::Normal},
        {u"Palette.Mid"_s, QPalette::Mid, QPalette::Normal},
        {u"Palette.Dark"_s, QPalette::Dark, QPalette::Normal},
        {u"Palette.Shadow"_s, QPalette::Shadow, QPalette::Normal},
        {u"Palette.WindowTextDisabled"_s, QPalette::WindowText, QPalette::Disabled},
        {u"Palette.TextDisabled"_s, QPalette::Text, QPalette::Disabled},
        {u"Palette.ToolTipTextDisabled"_s, QPalette::ToolTipText, QPalette::Disabled},
        {u"Palette.BrightTextDisabled"_s, QPalette::BrightText, QPalette::Disabled},
        {u"Palette.HighlightedTextDisabled"_s, QPalette::HighlightedText, QPalette::Disabled},
        {u"Palette.ButtonTextDisabled"_s, QPalette::ButtonText, QPalette::Disabled}
    };

    QPalette palette = QApplication::style()->standardPalette();
    for (const ColorDescriptor &colorDescriptor : paletteColorDescriptors)
    {
        // For backward compatibility, the palette color overrides are read from the section of the "light mode" colors
        const QColor newColor = m_themeSource->getColor(colorDescriptor.id, ColorMode::Light);
        if (newColor.isValid())
            palette.setColor(colorDescriptor.colorGroup, colorDescriptor.colorRole, newColor);
    }

    qApp->setPalette(palette);
}
