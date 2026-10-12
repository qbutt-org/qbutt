#ifndef QBT_QUICK_NATIVE_H
#define QBT_QUICK_NATIVE_H

#include <cstdint>
#include <memory>

#include <QString>
#include <QSize>

#include "rust/cxx.h"

namespace Qbutt
{
    class WorkModel;
    class VariantModel;
    class ChapterModel;
    class TextBlockModel;
    class SourceModel;
    class DownloadModel;
    class PageModel;
    struct RetiredReaderCache;
    struct Services;
    struct BrowserInstallation;
    enum class ContentError : std::uint8_t;
    enum class ReaderError : std::uint8_t;

    std::unique_ptr<WorkModel> createWorkModel();
    std::unique_ptr<VariantModel> createVariantModel();
    std::unique_ptr<ChapterModel> createChapterModel();
    std::unique_ptr<TextBlockModel> createTextBlockModel();
    std::unique_ptr<SourceModel> createSourceModel();
    std::unique_ptr<DownloadModel> createDownloadModel();
    QString sourceError(std::int32_t reason);
    std::unique_ptr<PageModel> createPageModel();
    QString contentError(ContentError error);
    void logContentError(const QString &operation, const QString &detail);
    QString formatSize(std::uint64_t bytes);
    QString contentLanguage();
    QString formatTranslationLabel(const QString &language, const QString &translation);
    QString formatVideoLabel(const QString &episode, const QString &quality, const QString &translation,
        const QString &player, const QString &source, const QString &title);
    QString readerError(ReaderError error);
    QString chapterTitle(const QString &number, const QString &volume, const QString &title);
    QSize validatedImageSize(const QString &path);
    void retireReaderCache(::rust::Box<RetiredReaderCache> cache);
    void retireContentServices(::rust::Box<Services> services);
    ::rust::Vec<BrowserInstallation> browserInstallations();
    bool openBrowserUrl(const QString &url, const QString &choice);
    QString browserDiscoveryPath();
    void copyShareUrl(const QString &url);
}

#endif
