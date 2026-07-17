// Generates byte-for-byte C4PXSSystem::Save fixtures for the Rust decoder
// tests. Run from this directory with:
//   c++ -std=c++20 generate_cpp_pxs_fixtures.cpp -o /tmp/generate-pxs
//   /tmp/generate-pxs .

#include <array>
#include <bit>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <stdexcept>

namespace {

constexpr std::size_t chunk_size = 500;
constexpr std::size_t chunk_count = 3;
constexpr std::size_t record_count = chunk_size * chunk_count;

struct FixedPxs {
  std::int32_t mat;
  std::int32_t x;
  std::int32_t y;
  std::int32_t xdir;
  std::int32_t ydir;
};

struct FloatPxs {
  std::int32_t mat;
  float x;
  float y;
  float xdir;
  float ydir;
};

static_assert(std::endian::native == std::endian::little);
static_assert(sizeof(FixedPxs) == 20);
static_assert(sizeof(FloatPxs) == 20);
static_assert(offsetof(FixedPxs, x) == 4);
static_assert(offsetof(FloatPxs, x) == 4);

template <typename Record>
void write_component(const std::filesystem::path &path, std::int32_t format,
                     const std::array<Record, record_count> &records) {
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  output.write(reinterpret_cast<const char *>(&format), sizeof(format));
  output.write(reinterpret_cast<const char *>(records.data()),
               sizeof(records));
  if (!output) {
    throw std::runtime_error("failed to write PXS fixture");
  }
}

} // namespace

int main(int argc, char **argv) {
  if (argc != 2) {
    return 2;
  }
  const std::filesystem::path output_dir(argv[1]);

  std::array<FixedPxs, record_count> fixed{};
  std::array<FloatPxs, record_count> floats{};
  for (std::size_t index = 0; index < record_count; ++index) {
    fixed[index].mat = -1;
    floats[index].mat = -1;
  }

  fixed[7] = {2, 78'643, -327'680, 32'768, -6'553};
  fixed[2 * chunk_size + 499] = {4, 78'643, -327'680, 32'768, -6'553};
  floats[7] = {2, 1.2F, -5.0F, 0.5F, -0.1F};
  floats[2 * chunk_size + 499] = {4, 1.2F, -5.0F, 0.5F, -0.1F};

  write_component(output_dir / "cpp_pxs_form1.c4b", 1, fixed);
  write_component(output_dir / "cpp_pxs_form2.c4b", 2, floats);
}
