// SPDX-License-Identifier: GPL-2.0-or-later
#include "qbutt_quick/native.h"

#include <utility>

#include <QClipboard>
#include <QCoreApplication>
#include <QGuiApplication>
#include <QImageReader>
#include <QLocale>
#include <QLoggingCategory>
#include <QStringList>
#include <QThreadPool>

#include "qbutt_quick/src/controller.cxxqt.h"
#include "qbutt_quick/src/models.cxxqt.h"
#include "qbutt_quick/src/reader_models.cxxqt.h"
#include "qbutt_quick/src/text_model.cxxqt.h"
#include "qbutt_quick/src/source_model.cxxqt.h"
#include "qbutt_quick/src/download_model.cxxqt.h"
#include "qbutt_quick/src/page_model.cxxqt.h"

Q_LOGGING_CATEGORY(contentLog, "qbutt.content")

void Qbutt::copyShareUrl(const QString &url)
{
    QGuiApplication::clipboard()->setText(url);
}

void Qbutt::retireContentServices(::rust::Box<Services> services)
{
    // The captured owner is destroyed on the pool thread. Runtime destruction
    // waits for blocking jobs while this runnable keeps Qt's shutdown drain open.
    QThreadPool::globalInstance()->start([services = std::move(services)] {});
}

void Qbutt::retireReaderCache(::rust::Box<RetiredReaderCache> cache)
{
    QThreadPool::globalInstance()->start([cache = std::move(cache)]
    {
        cleanRetiredReaderCache(*cache);
    });
}

std::unique_ptr<Qbutt::WorkModel> Qbutt::createWorkModel()
{
    return std::make_unique<WorkModel>();
}

std::unique_ptr<Qbutt::VariantModel> Qbutt::createVariantModel()
{
    return std::make_unique<VariantModel>();
}

std::unique_ptr<Qbutt::ChapterModel> Qbutt::createChapterModel()
{
    return std::make_unique<ChapterModel>();
}

std::unique_ptr<Qbutt::TextBlockModel> Qbutt::createTextBlockModel()
{
    return std::make_unique<TextBlockModel>();
}

std::unique_ptr<Qbutt::SourceModel> Qbutt::createSourceModel()
{
    return std::make_unique<SourceModel>();
}

std::unique_ptr<Qbutt::DownloadModel> Qbutt::createDownloadModel()
{
    return std::make_unique<DownloadModel>();
}

QString Qbutt::sourceError(const std::int32_t reason)
{
    switch (reason)
    {
    case 1:
        return QCoreApplication::translate("ContentController", "The source did not respond in time.");
    case 2:
        return QCoreApplication::translate("ContentController", "The source denied access.");
    case 3:
        return QCoreApplication::translate("ContentController", "This item is no longer available.");
    case 4:
        return QCoreApplication::translate("ContentController", "Could not load source settings.");
    case 5:
        return QCoreApplication::translate("ContentController", "Could not save source settings.");
    case 6:
        return QCoreApplication::translate("ContentController", "Sign in to this source to continue.");
    case 7:
        return QCoreApplication::translate("ContentController", "RuTracker needs a manual check.");
    case 8:
        return QCoreApplication::translate("ContentController", "Choose a browser under Options > Search > Browser.");
    case 9:
        return QCoreApplication::translate("ContentController", "Connect qcatch under Options > Search > Browser.");
    default:
        return QCoreApplication::translate("ContentController", "Could not get data from this source.");
    }
}

std::unique_ptr<Qbutt::PageModel> Qbutt::createPageModel()
{
    return std::make_unique<PageModel>();
}

QString Qbutt::contentLanguage()
{
    const QLocale locale;
    return locale.language() == QLocale::C ? QStringLiteral("en") : locale.name().section(u'_', 0, 0).toLower();
}

QString Qbutt::formatVideoLabel(const QString &episode, const QString &quality, const QString &translation,
    const QString &player, const QString &source, const QString &title)
{
    const QString label = !episode.isEmpty()
        ? QCoreApplication::translate("ContentController", "Episode %1").arg(episode)
        : !title.isEmpty() ? title : QCoreApplication::translate("ContentController", "Video");
    QString voice = translation;
    const QString subtitlePrefix = QStringLiteral("Субтитры");
    if ((voice == subtitlePrefix) || voice.startsWith(subtitlePrefix + u' '))
        voice.replace(0, subtitlePrefix.size(), QCoreApplication::translate("ContentController", "Subtitles"));

    QStringList details;
    for (const QString &detail : {voice, quality, player})
    {
        if (!detail.isEmpty() && (detail.compare(source, Qt::CaseInsensitive) != 0)
            && !details.contains(detail, Qt::CaseInsensitive))
        {
            details.append(detail);
        }
    }
    return details.isEmpty() ? label : QStringLiteral("%1 [%2]").arg(label, details.join(QStringLiteral(", ")));
}

QString Qbutt::formatTranslationLabel(const QString &language, const QString &translation)
{
    if (language.isEmpty())
        return translation.isEmpty() ? QCoreApplication::translate("ContentController", "Translation") : translation;
    const QLocale locale {language};
    QString label = locale.language() == QLocale::C ? language.toUpper() : locale.nativeLanguageName();
    if (!translation.isEmpty())
        label += QStringLiteral(" [") + translation + u']';
    return label;
}

QString Qbutt::readerError(const ReaderError error)
{
    switch (error)
    {
    case ReaderError::Chapters:
        return QCoreApplication::translate("ContentController", "Could not load chapters.");
    case ReaderError::Chapter:
        return QCoreApplication::translate("ContentController", "Could not load this chapter.");
    case ReaderError::Page:
        return QCoreApplication::translate("ContentController", "Could not load this page.");
    case ReaderError::PageUnavailable:
        return QCoreApplication::translate("ContentController", "This page is no longer available.");
    case ReaderError::ProgressWrite:
        return QCoreApplication::translate("ContentController", "Could not save reading position.");
    case ReaderError::ChapterUnavailable:
        return QCoreApplication::translate("ContentController", "This chapter is no longer available.");
    }
    Q_UNREACHABLE();
}

QString Qbutt::chapterTitle(const QString &number, const QString &volume, const QString &title)
{
    QString label;
    if (!number.isEmpty())
        label = QCoreApplication::translate("ContentController", "Chapter %1").arg(number);
    else if (!volume.isEmpty())
        label = QCoreApplication::translate("ContentController", "Volume %1").arg(volume);

    if (!title.isEmpty())
        return label.isEmpty() ? title : (label + QStringLiteral(" — ") + title);
    return label.isEmpty() ? QCoreApplication::translate("ContentController", "Chapter") : label;
}

QSize Qbutt::validatedImageSize(const QString &path)
{
    QImageReader reader {path};
    reader.setDecideFormatFromContent(true);
    const QSize size = reader.size();
    constexpr int dimensionLimit = 16000;
    constexpr qint64 pixelLimit = 40000000;
    const QByteArray format = reader.format();
    if (!size.isValid() || (size.width() > dimensionLimit) || (size.height() > dimensionLimit)
        || ((static_cast<qint64>(size.width()) * size.height()) > pixelLimit)
        || ((format != "jpeg") && (format != "jpg") && (format != "png")
            && (format != "webp") && (format != "gif") && (format != "bmp")))
    {
        return {};
    }
    const QImage image = reader.read();
    if (image.isNull() || (image.size() != size))
        return {};
    return size;
}

QString Qbutt::contentError(const ContentError error)
{
    switch (error)
    {
    case ContentError::ServicesUnavailable:
        return QCoreApplication::translate("ContentController", "Could not start content services.");
    case ContentError::Download:
        return QCoreApplication::translate("ContentController", "Could not start the download.");
    case ContentError::LibraryOpen:
        return QCoreApplication::translate("ContentController", "Could not open the content library.");
    case ContentError::HistoryRead:
        return QCoreApplication::translate("ContentController", "Could not load history.");
    case ContentError::ReaderSettingsWrite:
        return QCoreApplication::translate("ContentController", "Could not save reading settings.");
    case ContentError::Playback:
        return QCoreApplication::translate("ContentController", "Could not start playback.");
    case ContentError::PlaybackHistoryWrite:
        return QCoreApplication::translate("ContentController", "Could not save playback history.");
    case ContentError::PlaybackPending:
        return QCoreApplication::translate("ContentController", "Finish another playback or save pending history first.");
    case ContentError::ExternalPlayback:
        return QCoreApplication::translate("ContentController", "Playback from this external source is not available yet.");
    case ContentError::BrowserPlayback:
        return QCoreApplication::translate("ContentController", "Could not play the selected VK video. Check the browser connection and try again.");
    case ContentError::LibraryWrite:
        return QCoreApplication::translate("ContentController", "Could not save the content library.");
    }
    Q_UNREACHABLE();
}

QString Qbutt::formatSize(const std::uint64_t bytes)
{
    return QLocale().formattedDataSize(static_cast<qint64>(bytes));
}

void Qbutt::logContentError(const QString &operation, const QString &detail)
{
    qCWarning(contentLog).noquote() << operation << ':' << detail;
}
