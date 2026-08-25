// Mirrors StdJpeg::Impl (oracle src/StdJpegLibjpeg.cpp:38-141) and the
// C4Surface::ReadJPEG row loop (oracle src/C4Surface.cpp:1029-1072) so the
// exact partial-pixel behaviour on a truncated entropy stream can be read off
// the real libjpeg.
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <stdexcept>
#include <vector>

extern "C" {
#include <jpeglib.h>
}

struct StdJpegImpl {
    jpeg_decompress_struct cinfo;
    jpeg_error_mgr error_mgr;
    jpeg_source_mgr source_mgr;

    static constexpr JOCTET end_of_input[] = {0xff, JPEG_EOI};

    void *rowBuffer;

    StdJpegImpl(const void *const fileContents, const std::size_t fileSize) {
        cinfo.err = jpeg_std_error(&error_mgr);
        error_mgr.error_exit = [](const j_common_ptr cinfo) {
            char buffer[JMSG_LENGTH_MAX];
            cinfo->err->format_message(cinfo, buffer);
            throw std::runtime_error(buffer);
        };
        error_mgr.output_message = [](j_common_ptr) {};
        jpeg_create_decompress(&cinfo);

        cinfo.src = &source_mgr;
        source_mgr.next_input_byte = static_cast<const JOCTET *>(fileContents);
        source_mgr.bytes_in_buffer = fileSize;
        source_mgr.init_source = [](j_decompress_ptr) {};
        source_mgr.fill_input_buffer = [](const j_decompress_ptr cinfo) {
            cinfo->src->next_input_byte = end_of_input;
            cinfo->src->bytes_in_buffer = sizeof(end_of_input);
            return static_cast<boolean>(true);
        };
        source_mgr.skip_input_data = [](const j_decompress_ptr cinfo, const long num_bytes) {
            cinfo->src->next_input_byte += num_bytes;
            cinfo->src->bytes_in_buffer -= num_bytes;
            if (cinfo->src->bytes_in_buffer <= 0) {
                cinfo->src->next_input_byte = end_of_input;
                cinfo->src->bytes_in_buffer = sizeof(end_of_input);
            }
        };
        source_mgr.resync_to_restart = jpeg_resync_to_restart;
        source_mgr.term_source = [](j_decompress_ptr) {};

        jpeg_read_header(&cinfo, TRUE);
        cinfo.out_color_space = JCS_RGB;
        jpeg_start_decompress(&cinfo);

        const JDIMENSION samplesPerRow = cinfo.output_width * cinfo.output_components;
        rowBuffer = (*cinfo.mem->alloc_sarray)(reinterpret_cast<j_common_ptr>(&cinfo),
                                               JPOOL_IMAGE, samplesPerRow, 1);
    }

    ~StdJpegImpl() { jpeg_destroy_decompress(&cinfo); }

    void Finish() { jpeg_finish_decompress(&cinfo); }

    const void *DecodeRow() {
        if (cinfo.output_scanline >= cinfo.output_height) return nullptr;
        jpeg_read_scanlines(&cinfo, static_cast<JSAMPARRAY>(rowBuffer), 1);
        return static_cast<JSAMPARRAY>(rowBuffer)[0];
    }
};

constexpr JOCTET StdJpegImpl::end_of_input[];

int main(int argc, char **argv) {
    if (argc < 2) {
        std::fprintf(stderr, "usage: %s <file.jpg> [truncate_to_bytes]\n", argv[0]);
        return 2;
    }
    std::FILE *f = std::fopen(argv[1], "rb");
    if (!f) { std::perror("open"); return 2; }
    std::fseek(f, 0, SEEK_END);
    long size = std::ftell(f);
    std::fseek(f, 0, SEEK_SET);
    std::vector<unsigned char> data(static_cast<size_t>(size));
    if (std::fread(data.data(), 1, data.size(), f) != data.size()) { std::perror("read"); return 2; }
    std::fclose(f);
    if (argc >= 3) {
        size_t keep = std::strtoul(argv[2], nullptr, 10);
        if (keep < data.size()) data.resize(keep);
    }

    // C4Surface::ReadJPEG: the surface is created at full size, rows are
    // written as they decode, and a decode exception is caught and logged —
    // the function still returns true with whatever landed.
    std::vector<uint8_t> surface;
    uint32_t width = 0, height = 0;
    uint32_t rows_written = 0;
    const char *error = nullptr;
    std::string error_text;
    try {
        StdJpegImpl jpeg(data.data(), data.size());
        width = jpeg.cinfo.output_width;
        height = jpeg.cinfo.output_height;
        surface.assign(static_cast<size_t>(width) * height * 3, 0);
        for (uint32_t y = 0; y < height; ++y) {
            const auto row = jpeg.DecodeRow();
            const auto pixels = static_cast<const uint8_t *>(row);
            for (uint32_t x = 0; x < width; ++x) {
                surface[(static_cast<size_t>(y) * width + x) * 3 + 0] = pixels[x * 3 + 0];
                surface[(static_cast<size_t>(y) * width + x) * 3 + 1] = pixels[x * 3 + 1];
                surface[(static_cast<size_t>(y) * width + x) * 3 + 2] = pixels[x * 3 + 2];
            }
            rows_written = y + 1;
        }
        jpeg.Finish();
    } catch (const std::runtime_error &e) {
        error_text = e.what();
        error = error_text.c_str();
    }

    std::printf("{\"width\":%u,\"height\":%u,\"rows_written\":%u,\"bytes\":%zu,\"error\":\"%s\",\"rgb\":[",
                width, height, rows_written, data.size(), error ? error : "");
    for (size_t i = 0; i < surface.size(); ++i) {
        if (i) std::printf(",");
        std::printf("%u", surface[i]);
    }
    std::printf("]}\n");
    return 0;
}
