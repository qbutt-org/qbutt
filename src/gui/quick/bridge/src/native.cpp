// SPDX-License-Identifier: GPL-2.0-or-later
#include "qbutt_quick/native.h"

#include <utility>

#include <QCoreApplication>
#include <QImageReader>
#include <QLocale>
#include <QLoggingCategory>
#include <QThreadPool>
#include <QUrl>

#include "qbutt_quick/src/controller.cxxqt.h"
#include "qbutt_quick/src/models.cxxqt.h"
#include "qbutt_quick/src/reader_models.cxxqt.h"

Q_LOGGING_CATEGORY(contentLog, "qbutt.content")

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

QString Qbutt::contentLanguage()
{
    const QLocale locale;
    return locale.language() == QLocale::C ? QStringLiteral("en") : locale.name().section(u'_', 0, 0).toLower();
}

QString Qbutt::formatTranslationLabel(const QString &language, const QString &translation)
{
    const QLocale locale {language};
    QString label = locale.language() == QLocale::C ? language.toUpper() : locale.nativeLanguageName();
    if (!translation.isEmpty())
        label += QStringLiteral(" [") + translation + u']';
    return label;
}

QString Qbutt::readerError(const std::int32_t operation)
{
    switch (operation)
    {
    case 0:
        return QCoreApplication::translate("ContentController", "Could not load chapters.");
    case 1:
        return QCoreApplication::translate("ContentController", "Could not load this chapter.");
    case 2:
        return QCoreApplication::translate("ContentController", "Could not load this page.");
    case 4:
        return QCoreApplication::translate("ContentController", "This page is no longer available.");
    case 5:
        return QCoreApplication::translate("ContentController", "Could not save reading position.");
    default:
        return QCoreApplication::translate("ContentController", "This chapter is no longer available.");
    }
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

QString Qbutt::validatedImageUrl(const QString &path)
{
    QImageReader reader {path};
    reader.setDecideFormatFromContent(true);
    const QSize size = reader.size();
    constexpr int dimensionLimit = 16000;
    constexpr qint64 pixelLimit = 40000000;
    const QByteArray format = reader.format();
    if (!size.isValid() || (size.width() > dimensionLimit) || (size.height() > dimensionLimit)
        || ((static_cast<qint64>(size.width()) * size.height()) > pixelLimit)
        || ((format != "jpeg") && (format != "jpg") && (format != "png") && (format != "webp")))
    {
        return {};
    }
    const QImage image = reader.read();
    if (image.isNull() || (image.size() != size))
        return {};
    return QUrl::fromLocalFile(path).toString(QUrl::FullyEncoded);
}

QString Qbutt::contentError(const std::int32_t operation)
{
    switch (operation)
    {
    case 0:
        return QCoreApplication::translate("ContentController", "Could not start content services.");
    case 1:
        return QCoreApplication::translate("ContentController", "Search could not be completed.");
    case 2:
        return QCoreApplication::translate("ContentController", "Could not load variants.");
    case 3:
        return QCoreApplication::translate("ContentController", "Could not start the download.");
    case 4:
        return QCoreApplication::translate("ContentController", "Could not open the content library.");
    case 6:
        return QCoreApplication::translate("ContentController", "Could not load reading history.");
    default:
        return QCoreApplication::translate("ContentController", "Could not save the content library.");
    }
}

QString Qbutt::formatSize(const std::uint64_t bytes)
{
    return QLocale().formattedDataSize(static_cast<qint64>(bytes));
}

void Qbutt::logContentError(const QString &operation, const QString &detail)
{
    qCWarning(contentLog).noquote() << operation << ':' << detail;
}
